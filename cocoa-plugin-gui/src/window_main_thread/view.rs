//! The editor's GL view: an `NSOpenGLView` subclass that owns the
//! [`EditorApp`], the egui context, and the glow painter, translates
//! NSEvents into egui events, and paints frames from `drawRect:`.
//!
//! Everything here runs on the main thread (the class is
//! `MainThreadOnly`); the only cross-thread state is the [`SharedSize`]
//! mirror and the `alive` flag, both atomics.

// NSOpenGLView is deprecated-but-shipping; using it is the v1 rendering
// decision (plan §2), so its deprecation warnings carry no information.
#![allow(deprecated)]

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Instant;

use objc2::encode::{Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{msg_send, sel, AnyThread, ClassType, MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::{
    NSEvent, NSOpenGLPixelFormat, NSOpenGLView, NSTrackingArea, NSTrackingAreaOptions,
};
use objc2_foundation::{NSRect, NSRunLoop, NSRunLoopCommonModes, NSTimer};

use plugin_gui_core::repaint::{plan_repaint, repaint_due, RepaintPlan};
use plugin_gui_core::{CloseNotifier, EditorApp, EditorError, SharedSize};

use super::runtime_class::{self, RuntimeClass, RuntimeDefined};
use crate::input::{self, InputState};

/// The GL-side state built once in [`EditorView::init_gl`].
struct PaintState {
    gl: Arc<glow::Context>,
    painter: egui_glow::Painter,
    /// Keeps the OpenGL framework mapped for as long as glow holds
    /// function pointers into it.
    _lib: libloading::Library,
}

pub(super) struct ViewIvars {
    editor_id: u64,
    app: RefCell<Box<dyn EditorApp>>,
    egui_ctx: egui::Context,
    paint: RefCell<Option<PaintState>>,
    input: RefCell<InputState>,
    pending_events: RefCell<Vec<egui::Event>>,
    shared_size: SharedSize,
    /// Cleared when the editor dies (user close or teardown); the `Send`
    /// handle reads it to mirror the Wayland handle's send-failure
    /// semantics after its editor thread has exited.
    alive: Arc<AtomicBool>,
    /// Tells the host the window closed itself (PLG-01). Raised from
    /// [`EditorView::fire_on_close`], i.e. only on the user-close and
    /// panic paths; the handle disarms it before a host destroy.
    closed: CloseNotifier,
    start_time: Instant,
    /// Reentrancy guard: a modal run loop started inside `ui()` (an rfd /
    /// NSOpenPanel dialog) services the main queue and can deliver another
    /// `drawRect:` for this view while a frame is mid-build. Nested paints
    /// are skipped (macos-editor-plan.md item 3h).
    in_paint: Cell<bool>,
    /// Set when egui or input wants a frame; consumed by the display-link
    /// tick, which turns it into `setNeedsDisplay`.
    needs_repaint: Cell<bool>,
    /// `Some(deadline)` while egui has asked for a repaint at a future
    /// instant (`request_repaint_after` at or beyond the immediate
    /// threshold — see [`plugin_gui_core::repaint`]). The 60 Hz tick
    /// converts it into `needs_repaint` once due; each painted frame
    /// re-plans it from that frame's `repaint_delay` (the Cocoa analog
    /// of the Wayland runtime's `State::repaint_at`).
    repaint_at: Cell<Option<Instant>>,
    /// A user close arrived while a frame (or modal loop) was in flight;
    /// handled on the next tick instead.
    close_requested: Cell<bool>,
    /// `EditorApp::on_close` fires exactly once, whichever path closes.
    on_close_fired: Cell<bool>,
    repaint_timer: RefCell<Option<Retained<NSTimer>>>,
    tracking_area: RefCell<Option<Retained<NSTrackingArea>>>,
}

/// The editor's `NSOpenGLView` subclass. Its Objective-C class is
/// registered at runtime under a per-image name (see [`runtime_class`]),
/// so every plugin binary in the host process gets its own.
#[repr(C)]
pub(super) struct EditorView {
    superclass: NSOpenGLView,
}

// SAFETY: `EditorView` is a `#[repr(C)]` wrapper around its superclass,
// and an Objective-C object pointer like it.
unsafe impl RefEncode for EditorView {
    const ENCODING_REF: Encoding = NSOpenGLView::ENCODING_REF;
}

// SAFETY: a reference-counted Objective-C object.
unsafe impl Message for EditorView {}

// SAFETY: `class()` is a subclass of `Super` (registered by
// `runtime_class::register`), whose instances are main-thread-only like
// every NSView's; the struct is a `#[repr(C)]` wrapper around `Super`.
unsafe impl ClassType for EditorView {
    type Super = NSOpenGLView;
    type ThreadKind = dyn MainThreadOnly;
    /// The base name; the registered one carries a per-image suffix.
    const NAME: &'static str = "RESCocoaPluginEditorView";

    fn class() -> &'static AnyClass {
        Self::runtime_class().class
    }

    fn as_super(&self) -> &Self::Super {
        &self.superclass
    }

    const __INNER: () = ();
    type __SubclassingType = Self;
}

impl std::ops::Deref for EditorView {
    type Target = NSOpenGLView;

    fn deref(&self) -> &NSOpenGLView {
        &self.superclass
    }
}

// SAFETY: registered by `runtime_class::register::<Self>`; `#[repr(C)]`
// wrapper around `Super`.
unsafe impl RuntimeDefined for EditorView {
    type Ivars = ViewIvars;

    fn runtime_class() -> &'static RuntimeClass {
        static CLASS: OnceLock<RuntimeClass> = OnceLock::new();
        CLASS.get_or_init(|| runtime_class::register::<Self>(Self::NAME, register_view_methods))
    }
}

/// The overrides AppKit calls on the view. Each is a thin
/// `extern "C-unwind"` trampoline into the `EditorView` method of the
/// same name.
fn register_view_methods(builder: &mut ClassBuilder) {
    /// Top-left-origin coordinates, matching egui — AppKit then hands
    /// us view-local points that need no flipping.
    extern "C-unwind" fn is_flipped(_: &EditorView, _: Sel) -> Bool {
        Bool::YES
    }
    /// Key events come straight to the view; without this the window
    /// has no first responder and every keypress beeps.
    extern "C-unwind" fn accepts_first_responder(_: &EditorView, _: Sel) -> Bool {
        Bool::YES
    }
    extern "C-unwind" fn draw_rect(this: &EditorView, _: Sel, _dirty: NSRect) {
        this.paint();
    }
    /// Re-render at the new scale when the window moves to a display
    /// with a different `backingScaleFactor` (the Cocoa analog of the
    /// Wayland runtime's `scale_factor_changed`). The factor itself is
    /// re-read every frame in `paint`, so marking a repaint suffices.
    extern "C-unwind" fn view_did_change_backing_properties(this: &EditorView, _: Sel) {
        this.mark_repaint();
    }
    extern "C-unwind" fn on_repaint_timer(this: &EditorView, _: Sel, _timer: &NSTimer) {
        this.tick();
    }

    // ---- pointer ----

    extern "C-unwind" fn pointer_moved(this: &EditorView, _: Sel, event: &NSEvent) {
        this.pointer_moved(event);
    }
    extern "C-unwind" fn mouse_exited(this: &EditorView, _: Sel, event: &NSEvent) {
        this.with_input(event, |input, out| input.pointer_left(out));
    }
    extern "C-unwind" fn mouse_down(this: &EditorView, _: Sel, event: &NSEvent) {
        this.pointer_button(event, egui::PointerButton::Primary, true);
    }
    extern "C-unwind" fn mouse_up(this: &EditorView, _: Sel, event: &NSEvent) {
        this.pointer_button(event, egui::PointerButton::Primary, false);
    }
    extern "C-unwind" fn right_mouse_down(this: &EditorView, _: Sel, event: &NSEvent) {
        this.pointer_button(event, egui::PointerButton::Secondary, true);
    }
    extern "C-unwind" fn right_mouse_up(this: &EditorView, _: Sel, event: &NSEvent) {
        this.pointer_button(event, egui::PointerButton::Secondary, false);
    }
    extern "C-unwind" fn other_mouse_down(this: &EditorView, _: Sel, event: &NSEvent) {
        if let Some(btn) = input::map_other_button(event.buttonNumber() as i64) {
            this.pointer_button(event, btn, true);
        }
    }
    extern "C-unwind" fn other_mouse_up(this: &EditorView, _: Sel, event: &NSEvent) {
        if let Some(btn) = input::map_other_button(event.buttonNumber() as i64) {
            this.pointer_button(event, btn, false);
        }
    }
    extern "C-unwind" fn scroll_wheel(this: &EditorView, _: Sel, event: &NSEvent) {
        let dx = event.scrollingDeltaX();
        let dy = event.scrollingDeltaY();
        let precise = event.hasPreciseScrollingDeltas();
        this.with_input(event, |input, out| input.scroll(dx, dy, precise, out));
    }

    // ---- keyboard ----

    extern "C-unwind" fn key_down(this: &EditorView, _: Sel, event: &NSEvent) {
        this.key_event(event, true);
    }
    extern "C-unwind" fn key_up(this: &EditorView, _: Sel, event: &NSEvent) {
        this.key_event(event, false);
    }
    extern "C-unwind" fn flags_changed(this: &EditorView, _: Sel, event: &NSEvent) {
        this.with_input(event, |_input, _out| {});
    }

    // SAFETY: each function's signature matches its selector's.
    unsafe {
        builder.add_method(
            sel!(isFlipped),
            is_flipped as extern "C-unwind" fn(_, _) -> _,
        );
        builder.add_method(
            sel!(acceptsFirstResponder),
            accepts_first_responder as extern "C-unwind" fn(_, _) -> _,
        );
        builder.add_method(sel!(drawRect:), draw_rect as extern "C-unwind" fn(_, _, _));
        builder.add_method(
            sel!(viewDidChangeBackingProperties),
            view_did_change_backing_properties as extern "C-unwind" fn(_, _),
        );
        builder.add_method(
            sel!(onRepaintTimer:),
            on_repaint_timer as extern "C-unwind" fn(_, _, _),
        );
        builder.add_method(
            sel!(mouseMoved:),
            pointer_moved as extern "C-unwind" fn(_, _, _),
        );
        builder.add_method(
            sel!(mouseDragged:),
            pointer_moved as extern "C-unwind" fn(_, _, _),
        );
        builder.add_method(
            sel!(rightMouseDragged:),
            pointer_moved as extern "C-unwind" fn(_, _, _),
        );
        builder.add_method(
            sel!(otherMouseDragged:),
            pointer_moved as extern "C-unwind" fn(_, _, _),
        );
        builder.add_method(
            sel!(mouseExited:),
            mouse_exited as extern "C-unwind" fn(_, _, _),
        );
        builder.add_method(
            sel!(mouseDown:),
            mouse_down as extern "C-unwind" fn(_, _, _),
        );
        builder.add_method(sel!(mouseUp:), mouse_up as extern "C-unwind" fn(_, _, _));
        builder.add_method(
            sel!(rightMouseDown:),
            right_mouse_down as extern "C-unwind" fn(_, _, _),
        );
        builder.add_method(
            sel!(rightMouseUp:),
            right_mouse_up as extern "C-unwind" fn(_, _, _),
        );
        builder.add_method(
            sel!(otherMouseDown:),
            other_mouse_down as extern "C-unwind" fn(_, _, _),
        );
        builder.add_method(
            sel!(otherMouseUp:),
            other_mouse_up as extern "C-unwind" fn(_, _, _),
        );
        builder.add_method(
            sel!(scrollWheel:),
            scroll_wheel as extern "C-unwind" fn(_, _, _),
        );
        builder.add_method(sel!(keyDown:), key_down as extern "C-unwind" fn(_, _, _));
        builder.add_method(sel!(keyUp:), key_up as extern "C-unwind" fn(_, _, _));
        builder.add_method(
            sel!(flagsChanged:),
            flags_changed as extern "C-unwind" fn(_, _, _),
        );
    }
}

impl EditorView {
    fn ivars(&self) -> &ViewIvars {
        runtime_class::ivars(self)
    }

    /// Allocate and initialize the view. The GL pipeline is built later by
    /// [`EditorView::init_gl`] (it needs the pixel format's context, which
    /// `initWithFrame:pixelFormat:` creates).
    pub(super) fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        pixel_format: &NSOpenGLPixelFormat,
        app: Box<dyn EditorApp>,
        shared_size: SharedSize,
        alive: Arc<AtomicBool>,
        closed: CloseNotifier,
        editor_id: u64,
    ) -> Result<Retained<Self>, EditorError> {
        let this = runtime_class::alloc_with_ivars::<Self>(
            mtm,
            ViewIvars {
                editor_id,
                app: RefCell::new(app),
                egui_ctx: egui::Context::default(),
                paint: RefCell::new(None),
                input: RefCell::new(InputState::new()),
                pending_events: RefCell::new(Vec::new()),
                shared_size,
                alive,
                closed,
                start_time: Instant::now(),
                in_paint: Cell::new(false),
                needs_repaint: Cell::new(true),
                repaint_at: Cell::new(None),
                close_requested: Cell::new(false),
                on_close_fired: Cell::new(false),
                repaint_timer: RefCell::new(None),
                tracking_area: RefCell::new(None),
            },
        );
        // SAFETY: NSOpenGLView's designated initializer (not overridden
        // here) on a freshly allocated instance with its state set.
        let this: Option<Retained<Self>> =
            unsafe { msg_send![this, initWithFrame: frame, pixelFormat: pixel_format] };
        let this =
            this.ok_or_else(|| EditorError::Cocoa("NSOpenGLView init failed".to_string()))?;

        // Mouse-move and enter/exit tracking. `InVisibleRect` makes the
        // area follow the view's geometry, so one area installed here is
        // enough — no `updateTrackingAreas` override needed.
        let options = NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::ActiveAlways
            | NSTrackingAreaOptions::InVisibleRect;
        // SAFETY: owner (the view) outlives the tracking area — teardown
        // removes and drops the area before the view goes away.
        let owner: &AnyObject = &this;
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                this.bounds(),
                options,
                Some(owner),
                None,
            )
        };
        this.addTrackingArea(&area);
        *this.ivars().tracking_area.borrow_mut() = Some(area);

        Ok(this)
    }

    /// Build the GL side: current context, swap interval 0, glow, painter.
    /// Called once from `EditorMain::create`, before the ready-handshake
    /// completes, so failures surface from `Editor::new`.
    pub(super) fn init_gl(&self) -> Result<(), EditorError> {
        let ctx = self
            .openGLContext()
            .ok_or_else(|| EditorError::Cocoa("view has no NSOpenGLContext".to_string()))?;
        ctx.makeCurrentContext();
        crate::gl_context::set_swap_interval_zero(&ctx);
        let (gl, lib) = crate::gl_context::load_glow()?;
        let painter = egui_glow::Painter::new(gl.clone(), "", None, false)
            .map_err(|e| EditorError::GlLoad(e.to_string()))?;
        *self.ivars().paint.borrow_mut() = Some(PaintState {
            gl,
            painter,
            _lib: lib,
        });
        Ok(())
    }

    /// Start the 60 Hz main-run-loop timer that paces repaints. A tick
    /// with nothing to repaint is a few `Cell` reads, so the idle cost is
    /// negligible; painting itself is still gated on `needs_repaint`, the
    /// analog of the Wayland runtime's compositor frame-callback gate.
    ///
    /// `NSRunLoopCommonModes` matters twice: it keeps frames coming during
    /// live window resizing (event-tracking mode) and during modal loops
    /// (an rfd dialog opened from `ui()`), where the reentrancy guard —
    /// not the timer — is what prevents nested paints.
    ///
    /// A `CADisplayLink` from `NSView.displayLinkWithTarget:selector:`
    /// would pace on the actual display refresh, but never fired when
    /// driven from a plugin-style setup on this macOS version; the fixed
    /// timer is the dependable equivalent at editor frame rates.
    pub(super) fn start_repaint_timer(&self) {
        let target: &AnyObject = self;
        // SAFETY: `self` is a valid target and `onRepaintTimer:` is
        // defined on this class with the matching (id sender) signature;
        // the timer retains its target until `invalidate` in `teardown`.
        let timer = unsafe {
            NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                1.0 / 60.0,
                target,
                sel!(onRepaintTimer:),
                None,
                true,
            )
        };
        // SAFETY: main run loop + framework-provided mode statics.
        unsafe {
            NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
        }
        *self.ivars().repaint_timer.borrow_mut() = Some(timer);
    }

    /// Ask for a frame on the next tick (and nudge AppKit directly so
    /// input feels immediate rather than one display-link period late).
    pub(super) fn mark_repaint(&self) {
        self.ivars().needs_repaint.set(true);
        self.setNeedsDisplay(true);
    }

    /// Publish the view's logical size to the `Editor` handle. Called from
    /// the window delegate on every resize — user-driven or programmatic —
    /// which is what lets a host persist the size the user actually left
    /// the window at (the Cocoa analog of the Wayland configure feedback,
    /// ba todo #1337).
    pub(super) fn publish_size(&self) {
        let bounds = self.bounds();
        self.ivars()
            .shared_size
            .set((bounds.size.width as u32, bounds.size.height as u32));
        self.mark_repaint();
    }

    /// User clicked the window's close button (`windowShouldClose:`).
    /// Returns whether AppKit should proceed with the close now.
    pub(super) fn handle_close_request(&self) -> bool {
        if self.ivars().in_paint.get() {
            // Mid-frame (or inside a modal loop started from `ui()`):
            // defer to the next display-link tick.
            self.ivars().close_requested.set(true);
            return false;
        }
        // Keep the view alive through teardown: dropping the registry
        // entry releases what may be the last strong references to this
        // very object while one of its methods is still on the stack.
        let this = self.retain();
        this.fire_on_close();
        super::destroy_from_user_close(this.ivars().editor_id);
        true
    }

    /// One display-link tick: service a deferred close, else repaint if
    /// something asked for one.
    fn tick(&self) {
        if std::env::var_os("CPG_DEBUG").is_some() {
            static TICKS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
            if n <= 3 || n % 60 == 0 {
                tracing::info!("cpg: tick #{n}");
            }
        }
        let ivars = self.ivars();
        if ivars.in_paint.get() {
            return;
        }
        if ivars.close_requested.get() {
            ivars.close_requested.set(false);
            // Same keep-alive rationale as `handle_close_request`.
            let this = self.retain();
            this.fire_on_close();
            super::destroy(this.ivars().editor_id);
            return;
        }
        // A due egui repaint deadline (planned at the end of
        // `paint_inner`) becomes a repaint request; the timer's 60 Hz
        // cadence is the polling resolution.
        if repaint_due(Instant::now(), ivars.repaint_at.get()) {
            ivars.repaint_at.set(None);
            ivars.needs_repaint.set(true);
        }
        if ivars.needs_repaint.replace(false) {
            self.setNeedsDisplay(true);
        }
    }

    /// Invoke `EditorApp::on_close` exactly once, tolerating (impossible,
    /// but FFI-adjacent) reentrancy instead of panicking across it, then
    /// tell the host the window is closing on its own (PLG-01).
    ///
    /// `on_close` is plugin code running inside an Objective-C callback
    /// (`windowShouldClose:` or the repaint timer), so a panic in it is
    /// caught here: unwinding into AppKit aborts the host (PLG-03).
    fn fire_on_close(&self) {
        let ivars = self.ivars();
        if !ivars.on_close_fired.replace(true) {
            if let Ok(mut app) = ivars.app.try_borrow_mut() {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    app.on_close();
                }));
                if result.is_err() {
                    tracing::error!("cocoa-plugin-gui: the editor's on_close panicked; closing anyway");
                }
            }
        }
        ivars.alive.store(false, Ordering::Relaxed);
        ivars.closed.notify();
    }

    /// A panic escaped plugin code during a frame. The editor is unusable
    /// (its `EditorApp` may be mid-mutation), so stop painting it and close
    /// it through the same deferred path a user close mid-frame takes:
    /// the next tick fires `on_close`, notifies the host and tears the
    /// window down. Painting stops at once because `paint` checks `alive`.
    fn close_after_panic(&self) {
        tracing::error!("cocoa-plugin-gui: the editor's ui() panicked; closing the editor");
        let ivars = self.ivars();
        ivars.alive.store(false, Ordering::Relaxed);
        ivars.close_requested.set(true);
    }

    /// Release everything that must die on the main thread: the repaint
    /// timer (which retains this view as its target), the tracking area,
    /// and the GL pipeline. Called from `EditorMain::teardown`; the
    /// `EditorApp` itself drops with the view's ivars.
    pub(super) fn teardown(&self) {
        self.ivars().alive.store(false, Ordering::Relaxed);
        if let Some(timer) = self.ivars().repaint_timer.borrow_mut().take() {
            timer.invalidate();
        }
        if let Some(area) = self.ivars().tracking_area.borrow_mut().take() {
            self.removeTrackingArea(&area);
        }
        if let Some(mut paint) = self.ivars().paint.borrow_mut().take() {
            if let Some(ctx) = self.openGLContext() {
                ctx.makeCurrentContext();
            }
            paint.painter.destroy();
        }
    }

    // ---- input plumbing ----

    /// Update modifiers from the event, run `f` against the input state
    /// with the pending-event queue, and schedule a repaint.
    fn with_input(&self, event: &NSEvent, f: impl FnOnce(&mut InputState, &mut Vec<egui::Event>)) {
        let ivars = self.ivars();
        let mut input = ivars.input.borrow_mut();
        input.set_modifier_flags(event.modifierFlags().0 as u64);
        let mut out = ivars.pending_events.borrow_mut();
        f(&mut input, &mut out);
        drop((input, out));
        self.mark_repaint();
    }

    /// The event's location in view-local top-left-origin points.
    fn event_pos(&self, event: &NSEvent) -> (f32, f32) {
        let p = self.convertPoint_fromView(event.locationInWindow(), None);
        (p.x as f32, p.y as f32)
    }

    fn pointer_moved(&self, event: &NSEvent) {
        let (x, y) = self.event_pos(event);
        self.with_input(event, |input, out| input.pointer_moved(x, y, out));
    }

    fn pointer_button(&self, event: &NSEvent, button: egui::PointerButton, pressed: bool) {
        let (x, y) = self.event_pos(event);
        self.with_input(event, |input, out| {
            input.set_pointer_pos(x, y);
            input.pointer_button(button, pressed, out);
        });
    }

    fn key_event(&self, event: &NSEvent, pressed: bool) {
        let chars_im = event
            .charactersIgnoringModifiers()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let chars = event
            .characters()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let repeat = pressed && event.isARepeat();
        self.with_input(event, |input, out| {
            input.key(&chars_im, &chars, pressed, repeat, out)
        });
    }

    // ---- painting ----

    /// Build one egui frame and paint it. Runs inside `drawRect:`.
    fn paint(&self) {
        let ivars = self.ivars();
        if ivars.in_paint.get() || !ivars.alive.load(Ordering::Relaxed) {
            return;
        }
        // Keep the view alive for the whole frame: `ui()` may start a
        // modal run loop (an rfd dialog), and a modal loop services the
        // main queue — where a host `destroy()` can land and drop the
        // registry's strong references to this very object while this
        // method is still on the stack (plan item 3h). AppKit's display
        // machinery happens to hold its own references today (the 3h
        // negative control passed without this retain), but that is its
        // internal timing, not a contract — same rationale as the retains
        // in `handle_close_request` and `tick`. `paint_inner` tolerates
        // resuming on a torn-down view (its `paint` state is gone, so it
        // returns before touching GL).
        let _keep_alive = self.retain();
        ivars.in_paint.set(true);
        // `paint_inner` runs the plugin's `ui()` inside `drawRect:`. A
        // panic must not unwind into AppKit — that aborts the whole host,
        // where on Wayland the same bug only ends the editor thread
        // (PLG-03). Every RefCell borrow it takes is scoped inside it, so
        // they are released by the unwind before we get back here.
        let painted =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.paint_inner()));
        ivars.in_paint.set(false);
        if painted.is_err() {
            self.close_after_panic();
        }
    }

    fn paint_inner(&self) {
        let ivars = self.ivars();
        let Some(ctx) = self.openGLContext() else {
            return;
        };
        ctx.makeCurrentContext();

        let bounds = self.bounds();
        let (w, h) = (bounds.size.width as f32, bounds.size.height as f32);
        let backing = self.convertRectToBacking(bounds);
        let (pw, ph) = (backing.size.width as i32, backing.size.height as i32);
        if pw <= 0 || ph <= 0 {
            return;
        }
        let pixels_per_point = self
            .window()
            .map(|win| win.backingScaleFactor() as f32)
            .unwrap_or(1.0);

        // Provide pixels_per_point via the viewport info, not via
        // Context::set_pixels_per_point — the latter only takes effect on
        // the *next* pass, so the font atlas would be sized for the wrong
        // pp on the first frame (same rationale as the Wayland runtime).
        let mut viewports = egui::ViewportIdMap::default();
        viewports.insert(
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                native_pixels_per_point: Some(pixels_per_point),
                ..Default::default()
            },
        );

        let events = std::mem::take(&mut *ivars.pending_events.borrow_mut());
        let raw_input = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            viewports,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(w, h),
            )),
            events,
            modifiers: ivars.input.borrow().modifiers(),
            time: Some(ivars.start_time.elapsed().as_secs_f64()),
            focused: true,
            ..Default::default()
        };

        // The app borrow spans `ui()`, which may start a modal run loop
        // (rfd). Nested paints are already rejected by `in_paint`; nothing
        // else on the main thread touches `app`.
        let full_output = {
            let mut app = ivars.app.borrow_mut();
            ivars.egui_ctx.run_ui(raw_input, |ui| app.ui(ui))
        };

        let clipped_primitives = ivars
            .egui_ctx
            .tessellate(full_output.shapes, pixels_per_point);

        let Some(paint) = &mut *ivars.paint.borrow_mut() else {
            return;
        };
        use glow::HasContext;
        // SAFETY: plain GL state calls on the context made current above.
        unsafe {
            paint.gl.viewport(0, 0, pw, ph);
            paint.gl.clear_color(0.08, 0.08, 0.10, 1.0);
            paint.gl.clear(glow::COLOR_BUFFER_BIT);
        }
        paint.painter.paint_and_update_textures(
            [pw as u32, ph as u32],
            pixels_per_point,
            &clipped_primitives,
            &full_output.textures_delta,
        );

        // Dev-only frame dump (`CPG_DUMP_FRAME=<path>`, frame picked by
        // `CPG_DUMP_FRAME_AT`, 1-based) — same hook as the Wayland
        // runtime's `WPG_DUMP_FRAME`, captured before the buffer swap.
        let frame_n = {
            static FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
        };
        if let Ok(path) = std::env::var("CPG_DUMP_FRAME") {
            static DONE: AtomicBool = AtomicBool::new(false);
            let target = std::env::var("CPG_DUMP_FRAME_AT")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(1)
                .max(1);
            if frame_n >= target && !DONE.swap(true, Ordering::Relaxed) {
                let mut buf = vec![0u8; (pw * ph * 4) as usize];
                // SAFETY: reading back the framebuffer just painted, into
                // a buffer of exactly pw*ph RGBA texels.
                unsafe {
                    paint.gl.read_pixels(
                        0,
                        0,
                        pw,
                        ph,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelPackData::Slice(Some(&mut buf)),
                    );
                }
                let _ = super::debug::dump_ppm(&path, pw as u32, ph as u32, &buf);
            }
        }

        ctx.flushBuffer();

        // Dev-only synthetic close (`CPG_TEST_CLOSE_AT=<n>`, 1-based):
        // drives the real `performClose:` → `windowShouldClose:` →
        // `on_close` path once the n-th frame has painted, so the close
        // plumbing is verifiable without an external click injector — the
        // Cocoa analog of `WPG_TEST_CLOSE_AT` (there the synthetic click
        // hits the CSD button; here the button is AppKit's, so we invoke
        // it the way AppKit itself would).
        if let Some(target) = std::env::var("CPG_TEST_CLOSE_AT")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
        {
            static FIRED: AtomicBool = AtomicBool::new(false);
            // Keep frames coming while the hook is armed — a settled UI
            // stops repainting and the frame counter would stall short of
            // the target.
            ivars.needs_repaint.set(true);
            if frame_n >= target.max(1) && !FIRED.swap(true, Ordering::Relaxed) {
                tracing::info!("cpg: CPG_TEST_CLOSE_AT performing close at frame {frame_n}");
                if let Some(win) = self.window() {
                    use objc2_foundation::NSObjectNSDelayedPerforming;
                    // Deferred to the next run-loop turn: performClose
                    // re-enters this view's close path, which must not run
                    // inside `in_paint`.
                    //
                    // SAFETY: `performClose:` is a valid NSWindow selector
                    // taking one (nullable) id argument.
                    unsafe {
                        win.performSelector_withObject_afterDelay(
                            sel!(performClose:),
                            None,
                            0.0,
                        );
                    }
                }
            }
        }

        // Schedule the repaint egui asked for. Immediate requests go
        // out on the next tick (as before); finite longer delays become
        // a deadline `tick` fires when due — previously anything at or
        // over 50 ms was silently dropped, freezing low-rate animations
        // (the drums editor's 10 Hz meter) and egui's ~500 ms caret
        // blink. Same decision logic as the Wayland runtime.
        let repaint_after = full_output
            .viewport_output
            .values()
            .map(|v| v.repaint_delay)
            .min()
            .unwrap_or(std::time::Duration::from_millis(16));
        match plan_repaint(Instant::now(), repaint_after, ivars.repaint_at.get()) {
            RepaintPlan::Now => {
                ivars.needs_repaint.set(true);
                ivars.repaint_at.set(None);
            }
            RepaintPlan::At(at) => ivars.repaint_at.set(Some(at)),
            RepaintPlan::Idle => ivars.repaint_at.set(None),
        }
    }
}
