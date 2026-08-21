//! The main-thread half of the runtime: window construction, the editor
//! registry, and teardown.
//!
//! Everything in this module runs on the AppKit main thread. The public
//! [`Editor`](crate::editor::Editor) handle reaches it exclusively by
//! dispatching closures onto the main queue that look editors up by id in
//! [`REGISTRY`] — a thread-local, so nothing here needs locks and nothing
//! Objective-C ever crosses a thread.

mod debug;
mod delegate;
mod view;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use plugin_gui_core::{EditorApp, EditorError, EditorOptions, SharedSize};

use delegate::WindowDelegate;
use view::EditorView;

thread_local! {
    /// All live editors, keyed by the id their `Editor` handle carries.
    /// Main-thread only by construction: every access happens inside a
    /// closure dispatched onto the main queue (or inline when the caller
    /// already is the main thread).
    static REGISTRY: RefCell<HashMap<u64, EditorMain>> = RefCell::new(HashMap::new());
}

/// One live editor's main-thread state: the window, its GL view, and the
/// window delegate (held here because `NSWindow.delegate` is a weak
/// reference).
pub(crate) struct EditorMain {
    window: Retained<NSWindow>,
    view: Retained<EditorView>,
    _delegate: Retained<WindowDelegate>,
}

impl EditorMain {
    /// Create the window + view + delegate for one editor and register it.
    /// Runs on the main thread (asserted); called from `Editor::new` either
    /// inline or via a synchronous main-queue dispatch — this *is* the
    /// ready-handshake, so any window/GL failure surfaces as the
    /// `Editor::new` error.
    pub(crate) fn create(
        id: u64,
        app: Box<dyn EditorApp>,
        options: &EditorOptions,
        shared_size: SharedSize,
        alive: Arc<AtomicBool>,
    ) -> Result<(), EditorError> {
        let mtm = MainThreadMarker::new()
            .expect("EditorMain::create must run on the AppKit main thread");

        let (w, h) = options.initial_size;
        let content = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w as f64, h as f64));

        let pf = crate::gl_context::pixel_format(mtm)?;
        let view = EditorView::new(mtm, content, &pf, app, shared_size, alive, id)?;
        // Retina: render at the backing resolution, not 1x. Physical sizes
        // come from `convertRectToBacking` from then on. (Deprecated with
        // the rest of NSOpenGL — the v1 rendering decision, plan §2.)
        #[allow(deprecated)]
        view.setWantsBestResolutionOpenGLSurface(true);

        let mut style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable;
        if options.resizable {
            style |= NSWindowStyleMask::Resizable;
        }

        // SAFETY: standard NSWindow designated initializer; `defer: false`
        // creates the window device immediately so GL setup below has a
        // real window to attach to eventually.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                content,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: we manage the window's lifetime through the `Retained`
        // in the registry; AppKit must not release it behind our back on
        // close (the user-close path removes it from the registry itself).
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(&options.title));
        // `app_id` is a Wayland concept (compositor grouping); Cocoa has
        // no equivalent for a floating window and ignores it.
        let (min_w, min_h) = options.min_size;
        window.setContentMinSize(NSSize::new(min_w as f64, min_h as f64));
        window.setContentView(Some(&view));
        window.makeFirstResponder(Some(&view));
        window.center();

        let delegate = WindowDelegate::new(mtm, view.clone());
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

        // GL + painter, eagerly: a machine that can't build the pipeline
        // fails `Editor::new` (the Wayland runtime's EGL stage equivalent)
        // instead of opening a window it can never draw into.
        view.init_gl()?;
        view.start_repaint_timer();

        REGISTRY.with(|r| {
            r.borrow_mut().insert(
                id,
                EditorMain {
                    window,
                    view,
                    _delegate: delegate,
                },
            )
        });
        Ok(())
    }

    fn show(&self) {
        self.window.makeKeyAndOrderFront(None);
        self.view.mark_repaint();
    }

    fn hide(&self) {
        self.window.orderOut(None);
    }

    fn resize(&self, width: u32, height: u32) {
        self.window
            .setContentSize(NSSize::new(width as f64, height as f64));
        self.view.mark_repaint();
    }

    /// Release everything this editor holds. `close_window` is false only
    /// on the user-close path, where AppKit is about to close the window
    /// itself (we returned `true` from `windowShouldClose:`).
    fn teardown(self, close_window: bool) {
        self.view.teardown();
        self.window.setDelegate(None);
        if close_window {
            self.window.close();
        }
        // Dropping `self` releases the window, view, and delegate.
    }
}

/// Run `f` against a registered editor's window state, if it still exists.
/// A command arriving after user-close simply finds nothing — the same
/// no-op a Wayland command send after thread death collapses to.
pub(crate) fn with_editor(id: u64, f: impl FnOnce(&EditorMain)) {
    REGISTRY.with(|r| {
        if let Some(m) = r.borrow().get(&id) {
            f(m);
        }
    });
}

pub(crate) fn show(id: u64) {
    with_editor(id, EditorMain::show);
}

pub(crate) fn hide(id: u64) {
    with_editor(id, EditorMain::hide);
}

pub(crate) fn resize(id: u64, width: u32, height: u32) {
    with_editor(id, |m| m.resize(width, height));
}

/// Remove and tear down an editor (host-initiated destroy). Idempotent:
/// a second call — or a destroy after user-close — finds nothing.
pub(crate) fn destroy(id: u64) {
    if let Some(m) = REGISTRY.with(|r| r.borrow_mut().remove(&id)) {
        m.teardown(true);
    }
}

/// Remove and tear down an editor whose window AppKit is closing itself
/// (user clicked the close button). Called by the view's close path.
pub(crate) fn destroy_from_user_close(id: u64) {
    if let Some(m) = REGISTRY.with(|r| r.borrow_mut().remove(&id)) {
        m.teardown(false);
    }
}
