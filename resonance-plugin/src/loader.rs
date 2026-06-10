//! Shared infrastructure for plugins that load payloads (NAM models,
//! impulse responses, ...) on a background thread: the single-slot
//! mailbox that hands the result to the audio thread, and the
//! directory-rescan helper that keeps the editor's file list in sync
//! with the loaded path.

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

/// Single-slot handoff from a loader thread to the audio thread. The
/// loader [`post`](Mailbox::post)s a finished payload; the audio thread
/// collects it with a non-blocking [`try_take`](Mailbox::try_take) at
/// the top of `process`.
pub struct Mailbox<T>(Arc<Mutex<Option<T>>>);

impl<T> Mailbox<T> {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }

    /// Publish a payload, replacing any that hasn't been collected yet.
    pub fn post(&self, payload: T) {
        *self.0.lock() = Some(payload);
    }

    /// Non-blocking take for the audio thread: `None` when the slot is
    /// empty or the loader is mid-post (never blocks audio).
    pub fn try_take(&self) -> Option<T> {
        self.0.try_lock()?.take()
    }

    /// Blocking take — initialize-time only, before audio runs.
    pub fn take(&self) -> Option<T> {
        self.0.lock().take()
    }
}

impl<T> Clone for Mailbox<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Default for Mailbox<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Scan the directory containing `path` for files with `extension`,
/// publish the sorted list into `file_list`, and return the index of
/// `path` in the new list (0 if absent or `path` has no parent).
pub fn rescan_directory(path: &str, extension: &str, file_list: &Mutex<Vec<String>>) -> usize {
    if let Some(dir) = Path::new(path).parent() {
        let files = resonance_common::scan_directory(dir, extension);
        let idx = files.iter().position(|f| f == path).unwrap_or(0);
        *file_list.lock() = files;
        idx
    } else {
        0
    }
}
