//! The "editor closed itself" signal a runtime raises towards the host.
//!
//! When the user closes an editor from its own window (the Wayland CSD
//! button or `xdg_toplevel.close`, the Cocoa titlebar), or the editor dies
//! on its own (a panicking `ui()`), the runtime tears the window down
//! without the host having asked. CLAP expects the plugin to report that
//! through `clap_host_gui.closed()`, otherwise the host keeps believing
//! the editor is open (PLG-01). [`EditorApp::on_close`](crate::EditorApp)
//! is the *plugin's* hook for the same moment; this is the *host's*.
//!
//! The runtime owns one [`CloseNotifier`] per editor and calls
//! [`CloseNotifier::notify`] on those paths. Whoever holds the runtime's
//! `Editor` handle (in practice `resonance_plugin::editor_host`) installs
//! the callback with [`CloseNotifier::set_callback`], and
//! [`CloseNotifier::disarm`]s it before a host-initiated teardown, so a
//! destroy the host asked for is never reported back as a close.
//!
//! Thread-agnostic: the Wayland runtime notifies from its editor thread,
//! the Cocoa one from the AppKit main thread. The callback therefore runs
//! on whichever thread that is, and must only latch state.

use std::sync::{Arc, Mutex};

type Callback = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct Inner {
    /// The window went away on its own. Sticky.
    fired: bool,
    /// The host is tearing the editor down itself; no notification.
    disarmed: bool,
    callback: Option<Callback>,
}

/// One-shot, race-free delivery of "this editor closed itself".
///
/// Cheap to clone; every clone refers to the same signal.
#[derive(Clone, Default)]
pub struct CloseNotifier {
    inner: Arc<Mutex<Inner>>,
}

impl CloseNotifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install the callback. If the editor already closed itself before
    /// the callback arrived (a window closed within moments of creation),
    /// it runs immediately, on the calling thread. At most one callback
    /// ever runs; a second `set_callback` replaces an unfired first one.
    pub fn set_callback(&self, callback: impl FnOnce() + Send + 'static) {
        {
            let mut inner = lock(&self.inner);
            if inner.disarmed {
                return;
            }
            if !inner.fired {
                inner.callback = Some(Box::new(callback));
                return;
            }
        }
        callback();
    }

    /// Runtime side: the editor closed without the host asking. Runs the
    /// callback once; later calls, and calls after [`Self::disarm`], do
    /// nothing. The callback runs outside the lock.
    pub fn notify(&self) {
        let callback = {
            let mut inner = lock(&self.inner);
            if inner.disarmed || inner.fired {
                return;
            }
            inner.fired = true;
            inner.callback.take()
        };
        if let Some(callback) = callback {
            callback();
        }
    }

    /// Handle side: the host is destroying the editor. Drops any pending
    /// callback and makes every later [`Self::notify`] a no-op.
    pub fn disarm(&self) {
        let dropped = {
            let mut inner = lock(&self.inner);
            inner.disarmed = true;
            inner.callback.take()
        };
        drop(dropped);
    }

    /// Whether [`Self::notify`] has fired.
    pub fn has_fired(&self) -> bool {
        lock(&self.inner).fired
    }
}

/// A panic while holding this lock can only come from a callback's drop,
/// which runs outside it; recover from poisoning rather than propagate a
/// panic into FFI-adjacent teardown code.
fn lock(inner: &Mutex<Inner>) -> std::sync::MutexGuard<'_, Inner> {
    inner.lock().unwrap_or_else(|e| e.into_inner())
}
