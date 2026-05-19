//! Public [`Editor`] handle — the caller-facing API.

use std::sync::mpsc;
use std::thread::JoinHandle;

use smithay_client_toolkit::reexports::calloop::channel as calloop_channel;

use crate::app::EditorApp;
use crate::error::EditorError;
use crate::window_thread::{Command, EditorThread};

/// Options passed to [`Editor::new`].
#[derive(Debug, Clone)]
pub struct EditorOptions {
    pub title: String,
    /// Wayland `app_id` (reverse-DNS) for the editor window. The
    /// compositor uses this for taskbar grouping, `xdg-foreign`
    /// parenting, and window-rule matching. Plugins **must** override
    /// the default to a plugin-specific id; otherwise every editor
    /// hosted by this crate collides in the compositor.
    pub app_id: String,
    pub initial_size: (u32, u32),
    pub min_size: (u32, u32),
    pub resizable: bool,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            title: "Plugin Editor".to_string(),
            // Generic fallback. Callers should pass their own.
            app_id: "com.resonance.plugin".to_string(),
            initial_size: (800, 600),
            min_size: (400, 300),
            resizable: true,
        }
    }
}

/// A handle to a running editor window.
///
/// Dropping the handle without calling [`Editor::destroy`] will also stop the
/// editor thread. Commands are dispatched asynchronously — returning from a
/// method does not guarantee the command has been processed by the editor
/// thread yet.
pub struct Editor {
    sender: calloop_channel::Sender<Command>,
    thread: Option<JoinHandle<()>>,
    size: (u32, u32),
    resizable: bool,
}

impl Editor {
    /// Create (but do not show) an editor window. Spawns the editor thread.
    pub fn new<A: EditorApp>(app: A, options: EditorOptions) -> Result<Self, EditorError> {
        let size = options.initial_size;
        let resizable = options.resizable;

        let (sender, cmd_channel) = calloop_channel::channel::<Command>();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), EditorError>>(1);

        let thread_opts = options.clone();
        let thread = std::thread::Builder::new()
            .name("wayland-plugin-gui".to_string())
            .spawn(move || {
                EditorThread::run(Box::new(app), thread_opts, cmd_channel, ready_tx);
            })
            .map_err(EditorError::ThreadSpawn)?;

        // Wait for the thread to finish initialisation (or fail).
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                // Thread set up an error; let it join and surface the error.
                let _ = thread.join();
                return Err(err);
            }
            Err(_) => return Err(EditorError::ChannelClosed),
        }

        Ok(Self {
            sender,
            thread: Some(thread),
            size,
            resizable,
        })
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
    pub fn set_size(&self, width: u32, height: u32) -> Result<(), EditorError> {
        self.sender
            .send(Command::Resize(width, height))
            .map_err(|_| EditorError::ChannelClosed)
    }

    pub fn get_size(&self) -> (u32, u32) {
        self.size
    }

    pub fn is_resizable(&self) -> bool {
        self.resizable
    }

    /// Request an immediate repaint on the next loop iteration.
    pub fn request_repaint(&self) {
        let _ = self.sender.send(Command::Repaint);
    }

    /// Stop the editor thread and destroy the window. Blocks until the thread
    /// joins.
    pub fn destroy(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let _ = self.sender.send(Command::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
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
