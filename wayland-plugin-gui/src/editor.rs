//! Public [`Editor`] handle — the caller-facing API.

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use plugin_gui_core::CloseNotifier;
use smithay_client_toolkit::reexports::calloop::channel as calloop_channel;

use crate::app::EditorApp;
use crate::error::EditorError;
use crate::join::join_with_timeout;
use crate::size::SharedSize;
use crate::window_thread::{Command, EditorThread};

// The options struct lives in `plugin-gui-core` (shared with every
// runtime); re-exported here so `crate::editor::EditorOptions` paths
// inside this crate keep resolving.
pub use plugin_gui_core::EditorOptions;

/// How long teardown waits for the editor thread before detaching it.
///
/// Generous next to any healthy teardown — a clean quit joins in
/// milliseconds, and even a stalled compositor is written off after
/// 250 ms (`FRAME_CALLBACK_STALL`) — but bounded, because the CLAP host
/// destroys editors on the audio-engine control thread: a plugin whose
/// `ui()` never returns (a modal rfd dialog is the known case — the
/// wedge the cocoa runtime's `modal_reentrancy` test guards against)
/// must cost that thread two seconds once, not wedge it forever.
const DESTROY_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

/// A handle to a running editor window.
///
/// Dropping the handle without calling [`Editor::destroy`] will also stop the
/// editor thread. Commands are dispatched asynchronously — returning from a
/// method does not guarantee the command has been processed by the editor
/// thread yet.
pub struct Editor {
    sender: calloop_channel::Sender<Command>,
    thread: Option<JoinHandle<()>>,
    /// The window's live size, published by the editor thread on every
    /// size it applies (see [`crate::size`]).
    size: SharedSize,
    resizable: bool,
    /// Raised by the editor thread when the window goes away without a
    /// host `destroy` (user close, or the thread dying); disarmed by
    /// [`Editor::stop`] so a host-initiated teardown is never reported.
    closed: CloseNotifier,
}

impl Editor {
    /// Create (but do not show) an editor window. Spawns the editor thread.
    pub fn new<A: EditorApp>(app: A, options: EditorOptions) -> Result<Self, EditorError> {
        let size = SharedSize::new(options.initial_size);
        let resizable = options.resizable;

        let closed = CloseNotifier::new();
        let (sender, cmd_channel) = calloop_channel::channel::<Command>();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), EditorError>>(1);

        let thread_opts = options.clone();
        let thread_size = size.clone();
        let thread_closed = closed.clone();
        let thread = std::thread::Builder::new()
            .name("wayland-plugin-gui".to_string())
            .spawn(move || {
                EditorThread::run(
                    Box::new(app),
                    thread_opts,
                    cmd_channel,
                    ready_tx,
                    thread_size,
                    thread_closed,
                );
            })
            .map_err(EditorError::ThreadSpawn)?;

        // Wait for the thread to finish initialisation (or fail).
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                // Thread hit a setup error and is exiting; reap it —
                // bounded all the same, so a teardown wedge inside the
                // failing thread cannot block plugin instantiation.
                let _ = join_with_timeout(thread, DESTROY_JOIN_TIMEOUT);
                return Err(err);
            }
            Err(_) => return Err(EditorError::ChannelClosed),
        }

        Ok(Self {
            sender,
            thread: Some(thread),
            size,
            resizable,
            closed,
        })
    }

    /// Install the callback that tells the host the window closed itself:
    /// the user closed it (CSD close button or `xdg_toplevel.close`, after
    /// [`EditorApp::on_close`]) or the editor thread died. Runs at most
    /// once, on the editor thread, after the window is gone; never for a
    /// teardown the caller started with [`Editor::destroy`] or a drop.
    /// If the window already closed, it runs immediately on this thread.
    pub fn set_closed_callback(&self, callback: impl FnOnce() + Send + 'static) {
        self.closed.set_callback(callback);
    }

    /// Show the window. Idempotent.
    pub fn show(&self) {
        let _ = self.sender.send(Command::Show);
    }

    /// Hide the window. Idempotent.
    pub fn hide(&self) {
        let _ = self.sender.send(Command::Hide);
    }

    /// Request the window be resized.
    ///
    /// On success the handle's bookkeeping is updated immediately, so a
    /// following [`Editor::get_size`] returns the requested size even
    /// though the editor thread applies the resize asynchronously. The
    /// compositor has the last word: whatever size it configures the
    /// window to — which may not be the one asked for — replaces this
    /// value as soon as the editor thread applies it.
    pub fn set_size(&mut self, width: u32, height: u32) -> Result<(), EditorError> {
        self.sender
            .send(Command::Resize(width, height))
            .map_err(|_| EditorError::ChannelClosed)?;
        self.size.set((width, height));
        Ok(())
    }

    /// The window's current logical size.
    ///
    /// This tracks what the window *is*, not what was last asked for:
    /// the editor thread publishes every size it applies, so a resize the
    /// user performed through the compositor (dragging an edge, tiling,
    /// maximising) shows up here. That is what makes a host persist and
    /// restore the size the user actually left the editor at — before ba
    /// todo #1337 this returned the last requested size and every
    /// interactive resize was lost on save.
    pub fn get_size(&self) -> (u32, u32) {
        self.size.get()
    }

    pub fn is_resizable(&self) -> bool {
        self.resizable
    }

    /// Stop the editor thread and destroy the window.
    ///
    /// Blocks until the thread joins, bounded by a wall-clock watchdog:
    /// the CLAP host calls this on the audio-engine control thread, and
    /// an unbounded join there would wedge the whole `AudioCommand`
    /// queue if the plugin's `ui()` never returns. On timeout the
    /// editor thread is left detached (leaked, by design — same
    /// trade-off as `AudioEngine::shutdown`) and this returns anyway.
    pub fn destroy(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        // Host-initiated: whatever the thread does on its way out is not
        // a close the host needs to hear about.
        self.closed.disarm();
        // The command channel is a calloop channel: sending pings the
        // event loop's wakeup fd, so an editor thread idle-parked in
        // `dispatch` sees Quit immediately — no separate wake is
        // needed. The one case no wake can reach is a plugin blocked
        // inside its own `ui()` (e.g. a modal dialog's nested run
        // loop); that is what the bounded join below is for.
        let _ = self.sender.send(Command::Quit);
        if let Some(thread) = self.thread.take() {
            if !join_with_timeout(thread, DESTROY_JOIN_TIMEOUT) {
                // Not the RT audio thread — teardown may report like
                // its neighbours (see EditorThread::run) do.
                eprintln!(
                    "wayland-plugin-gui: editor thread did not exit within {:?} \
                     (plugin ui() blocked?); detaching it instead of wedging \
                     the host thread",
                    DESTROY_JOIN_TIMEOUT
                );
            }
        }
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.stop();
        }
    }
}
