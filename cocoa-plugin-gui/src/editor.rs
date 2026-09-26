//! Public [`Editor`] handle — the caller-facing API.
//!
//! Same surface as `wayland_plugin_gui::Editor`, different transport: the
//! Wayland handle sends commands over a channel to a dedicated editor
//! thread; this one dispatches closures onto the AppKit **main queue**,
//! where the window controller lives (see `window_main_thread`). The handle
//! itself owns no Objective-C objects — only the registry id, the
//! [`SharedSize`] mirror, and the liveness flag — which is what makes it
//! honestly `Send` for the CLAP bridge.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;

use plugin_gui_core::{CloseNotifier, EditorApp, EditorError, EditorOptions, SharedSize};

use crate::window_main_thread;

/// Registry ids for live editors. Never reused within a process, so a
/// stale handle can only ever miss the registry, not address a stranger.
static NEXT_EDITOR_ID: AtomicU64 = AtomicU64::new(1);

/// Run `work` on the main thread and wait for it. Inline when we already
/// are the main thread — a synchronous dispatch to yourself deadlocks.
///
/// This wait is the runtime's wedge point (macos-editor-plan.md §1): if the
/// main thread never services its queue, the caller blocks — the same
/// failure mode as the Wayland handle's thread-join, and what the
/// `editor_open`-style watchdog tests exist to catch.
fn run_on_main_blocking<F: FnOnce() + Send>(work: F) {
    if MainThreadMarker::new().is_some() {
        work();
    } else {
        DispatchQueue::main().exec_sync(work);
    }
}

/// Run `work` on the main thread without waiting.
fn run_on_main_async<F: FnOnce() + Send + 'static>(work: F) {
    if MainThreadMarker::new().is_some() {
        work();
    } else {
        DispatchQueue::main().exec_async(work);
    }
}

/// A handle to a running editor window.
///
/// Dropping the handle without calling [`Editor::destroy`] also destroys
/// the window. Commands are dispatched asynchronously — returning from a
/// method does not guarantee the command has been processed by the main
/// thread yet.
pub struct Editor {
    id: u64,
    /// The window's live size, published by the main-thread controller on
    /// every size it applies (see `plugin_gui_core::size`).
    size: SharedSize,
    resizable: bool,
    /// Cleared by the controller when the editor dies (user close or
    /// teardown). Mirrors the Wayland handle's send-failure signal.
    alive: Arc<AtomicBool>,
    /// Raised by the main-thread controller when the window goes away
    /// without a host `destroy` (user close, or a panicking `ui()`);
    /// disarmed by [`Editor::stop`] so a host-initiated teardown is never
    /// reported.
    closed: CloseNotifier,
    /// Teardown ran (destroy or drop); makes both idempotent.
    stopped: bool,
}

impl Editor {
    /// Create (but do not show) an editor window.
    ///
    /// Callable from any thread. Window and GL construction run on the
    /// main thread — synchronously, mirroring the Wayland runtime's
    /// ready-handshake — so a failure there is returned from here.
    pub fn new<A: EditorApp>(app: A, options: EditorOptions) -> Result<Self, EditorError> {
        let id = NEXT_EDITOR_ID.fetch_add(1, Ordering::Relaxed);
        let size = SharedSize::new(options.initial_size);
        let resizable = options.resizable;
        let alive = Arc::new(AtomicBool::new(true));
        let closed = CloseNotifier::new();

        let mut result: Option<Result<(), EditorError>> = None;
        {
            let size = size.clone();
            let alive = Arc::clone(&alive);
            let closed = closed.clone();
            let app: Box<dyn EditorApp> = Box::new(app);
            let options = &options;
            let result = &mut result;
            run_on_main_blocking(move || {
                *result = Some(window_main_thread::EditorMain::create(
                    id, app, options, size, alive, closed,
                ));
            });
        }
        match result {
            Some(Ok(())) => Ok(Self {
                id,
                size,
                resizable,
                alive,
                closed,
                stopped: false,
            }),
            Some(Err(err)) => Err(err),
            // The dispatch never ran its closure — nothing left to talk to.
            None => Err(EditorError::ChannelClosed),
        }
    }

    /// Show the window. Idempotent.
    pub fn show(&self) {
        let id = self.id;
        run_on_main_async(move || window_main_thread::show(id));
    }

    /// Hide the window. Idempotent.
    pub fn hide(&self) {
        let id = self.id;
        run_on_main_async(move || window_main_thread::hide(id));
    }

    /// Request the window be resized.
    ///
    /// On success the handle's bookkeeping is updated immediately, so a
    /// following [`Editor::get_size`] returns the requested size even
    /// though the main thread applies the resize asynchronously. AppKit
    /// has the last word (min-size clamping, zoom): whatever size the
    /// window actually takes replaces this value as soon as it is applied.
    pub fn set_size(&mut self, width: u32, height: u32) -> Result<(), EditorError> {
        if !self.alive.load(Ordering::Relaxed) {
            // The editor died (user close) — same signal as the Wayland
            // handle's send to a hung-up channel.
            return Err(EditorError::ChannelClosed);
        }
        let id = self.id;
        run_on_main_async(move || window_main_thread::resize(id, width, height));
        self.size.set((width, height));
        Ok(())
    }

    /// The window's current logical size.
    ///
    /// This tracks what the window *is*, not what was last asked for: the
    /// controller publishes every size it applies, so a resize the user
    /// performed by dragging the window edge shows up here — which is what
    /// lets a host persist and restore the size the user actually left the
    /// editor at (ba todo #1337).
    pub fn get_size(&self) -> (u32, u32) {
        self.size.get()
    }

    pub fn is_resizable(&self) -> bool {
        self.resizable
    }

    /// Install the callback that tells the host the window closed itself:
    /// the user clicked the titlebar close button (after
    /// [`EditorApp::on_close`]) or the plugin's `ui()` panicked and the
    /// runtime closed the editor. Runs at most once, on the main thread;
    /// never for a teardown the caller started with [`Editor::destroy`] or
    /// a drop. If the window already closed, it runs immediately on this
    /// thread.
    pub fn set_closed_callback(&self, callback: impl FnOnce() + Send + 'static) {
        self.closed.set_callback(callback);
    }

    /// Destroy the window.
    ///
    /// On the main thread the teardown runs inline, before this returns.
    /// From any other thread it is queued onto the main queue and this
    /// returns at once (PLG-05): a synchronous dispatch would block until
    /// the main thread services its queue, and Resonance's quit path has
    /// the main thread waiting on exactly the engine thread that destroys
    /// editors — every quit with an editor open burned the whole shutdown
    /// deadline, and plugin teardown then raced process exit. A late
    /// teardown is safe: the handle owns no Objective-C objects, and the
    /// registry id makes a teardown that finds the editor already gone a
    /// no-op. (A spec-following CLAP host calls `gui.destroy` on the main
    /// thread anyway, where nothing changes.)
    pub fn destroy(mut self) {
        self.stop();
    }

    /// The flag the main-thread controller clears once the editor is torn
    /// down (or closed by the user). For tests that must wait for an
    /// asynchronous [`Editor::destroy`] to land; not part of the API.
    #[doc(hidden)]
    pub fn liveness(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.alive)
    }

    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        // Host-initiated: not a close the host needs to hear about.
        self.closed.disarm();
        let id = self.id;
        run_on_main_async(move || window_main_thread::destroy(id));
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        self.stop();
    }
}
