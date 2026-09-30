//! The editor's library work that must not run on the editor thread:
//! rescans (hashing), imports (copying + hashing), deletes and relinks
//! (which wait on the shared library's writer lock behind any of those).
//!
//! One job at a time on one helper thread, joined when the editor goes
//! away, so no thread of ours outlives the plugin image. The frame polls
//! [`Jobs::poll`] and applies the outcome (a notice, a selection, a load).

use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;

use parking_lot::Mutex;
use resonance_common::nam_library::{Entry, ImportOutcome, ScanReport};

/// What a finished job reports back to the frame.
pub(crate) enum JobDone {
    Rescanned(Result<ScanReport, String>),
    /// Imports; `load_single` loads the one file of a single-file import.
    Imported {
        results: Vec<Result<ImportOutcome, String>>,
        load_single: bool,
    },
    Deleted {
        name: String,
        result: Result<(), String>,
    },
    /// A located or at-path file brought into the library for the missing
    /// banner: the entry to load.
    Relinked(Result<(Entry, bool), String>),
    /// A file picked with Locate…, and its content id.
    Located(PathBuf, Option<String>),
}

#[derive(Default)]
pub(crate) struct Jobs {
    handle: Option<JoinHandle<()>>,
    done: Arc<Mutex<Option<JobDone>>>,
    /// What the running job is, for the footer.
    label: Option<String>,
}

impl Jobs {
    pub(crate) fn busy(&self) -> bool {
        self.handle.is_some()
    }

    pub(crate) fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Start `work` unless a job is running (then `false`).
    pub(crate) fn start(
        &mut self,
        label: impl Into<String>,
        work: impl FnOnce() -> JobDone + Send + 'static,
    ) -> bool {
        if self.busy() {
            return false;
        }
        let done = self.done.clone();
        match std::thread::Builder::new()
            .name("amp-library-job".into())
            .spawn(move || {
                let out = work();
                *done.lock() = Some(out);
            }) {
            Ok(h) => {
                self.handle = Some(h);
                self.label = Some(label.into());
                true
            }
            Err(e) => {
                tracing::warn!("could not start a library job: {e}");
                false
            }
        }
    }

    /// The outcome of a finished job, once.
    pub(crate) fn poll(&mut self) -> Option<JobDone> {
        if self.handle.as_ref().is_some_and(|h| h.is_finished()) {
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
            self.label = None;
        }
        if self.handle.is_none() {
            self.done.lock().take()
        } else {
            None
        }
    }

    /// Block until the running job (if any) is done, and return its
    /// outcome. For teardown and tests.
    pub(crate) fn wait(&mut self) -> Option<JobDone> {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.label = None;
        self.done.lock().take()
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
