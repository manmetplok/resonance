//! The Library overlay's work that must not run on the editor thread:
//! rescans (hashing manifests, measuring sizes), imports (copying a kit,
//! gigabytes) and deletes (`remove_dir_all` of a kit). Modelled on the
//! amp's `editor/jobs.rs`.
//!
//! One job at a time, each on a thread of its own. The frame polls
//! [`Jobs::poll`] and applies the outcome. When the editor goes away the
//! running job's cancel flag is raised — an import stops at its next
//! chunk, a size measurement between kits — and its thread is *detached*,
//! not joined: everything it touches is behind an `Arc`, and joining on
//! the UI thread would hold the host's editor-close for as long as a
//! multi-gigabyte copy or a `remove_dir_all` takes.
//!
//! The import's file dialog is not a job: it runs on its own thread
//! ([`Picker`]) so the editor thread never blocks in a modal dialog, and
//! an editor closed while it is up is not held open by it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use parking_lot::Mutex;
use resonance_common::drumkit_library::{ImportOutcome, ImportProgress};

/// What a finished job reports back to the frame.
pub(crate) enum JobDone {
    Rescanned {
        result: Result<(), String>,
        /// Another writer (a download installing, another editor's job)
        /// held the library, so nothing was scanned.
        skipped: bool,
        /// The user clicked Rescan (report the outcome), rather than the
        /// editor opening or the freshness poll.
        user: bool,
    },
    Imported(Box<Result<ImportOutcome, String>>),
    Deleted {
        name: String,
        result: Result<(), String>,
        /// Where the deleted row sat in the view, so the selection moves
        /// to its neighbour instead of to nothing.
        view_pos: Option<usize>,
    },
    /// The lazy missing-files check of one kit
    /// (`Library::check_missing_files`).
    CheckedFiles {
        id: String,
        result: Result<usize, String>,
    },
    /// A folder picked to relink a missing kit, with its manifest's id
    /// (or why it holds no kit) — `missing_kit.rs` decides what next.
    Located {
        dir: PathBuf,
        id: Result<String, String>,
    },
}

/// What kind of job is running. A [`JobKind::Check`] is a quick `stat`
/// pass the editor starts on its own; it does not block the Delete
/// confirm or the Import / Rescan buttons (what they start waits for it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobKind {
    Scan,
    Import,
    Delete,
    Check,
}

/// What a running job can see: its cancel flag and a progress slot the
/// footer draws.
#[derive(Clone)]
pub(crate) struct JobCtx {
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) progress: Arc<Mutex<Option<ImportProgress>>>,
}

#[derive(Default)]
pub(crate) struct Jobs {
    handle: Option<JoinHandle<()>>,
    done: Arc<Mutex<Option<JobDone>>>,
    /// What the running job is, for the footer.
    label: Option<String>,
    /// Whether the running job offers Cancel.
    cancellable: bool,
    kind: Option<JobKind>,
    ctx: Option<JobCtx>,
}

impl Jobs {
    pub(crate) fn busy(&self) -> bool {
        self.handle.is_some()
    }

    /// Whether a job that writes the library (scan, import, delete) is
    /// running — not just the editor's own quick missing-files check.
    pub(crate) fn writing(&self) -> bool {
        self.busy() && self.kind != Some(JobKind::Check)
    }

    pub(crate) fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub(crate) fn cancellable(&self) -> bool {
        self.busy() && self.cancellable
    }

    /// The running job's progress, if it reports any.
    pub(crate) fn progress(&self) -> Option<ImportProgress> {
        self.ctx.as_ref().and_then(|c| *c.progress.lock())
    }

    /// Ask the running job to stop at its next chunk.
    pub(crate) fn cancel(&self) {
        if let Some(c) = &self.ctx {
            c.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Start `work` unless a job is running (then `false`).
    pub(crate) fn start(
        &mut self,
        kind: JobKind,
        label: impl Into<String>,
        cancellable: bool,
        work: impl FnOnce(&JobCtx) -> JobDone + Send + 'static,
    ) -> bool {
        if self.busy() {
            return false;
        }
        let ctx = JobCtx {
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Mutex::new(None)),
        };
        let done = self.done.clone();
        let thread_ctx = ctx.clone();
        match std::thread::Builder::new()
            .name("drums-library-job".into())
            .spawn(move || {
                let out = work(&thread_ctx);
                *done.lock() = Some(out);
            }) {
            Ok(h) => {
                self.handle = Some(h);
                self.label = Some(label.into());
                self.cancellable = cancellable;
                self.kind = Some(kind);
                self.ctx = Some(ctx);
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
            self.kind = None;
            self.ctx = None;
        }
        if self.handle.is_none() {
            self.done.lock().take()
        } else {
            None
        }
    }

    /// Block until the running job (if any) is done, and return its
    /// outcome. For tests — only `DrumsEditorApp::finish_jobs` (in turn
    /// only `TestEditor`, `test-hooks`) calls it.
    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub(crate) fn wait(&mut self) -> Option<JobDone> {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.label = None;
        self.kind = None;
        self.ctx = None;
        self.done.lock().take()
    }
}

impl Drop for Jobs {
    /// Raise the cancel flag and detach the thread (see the module docs):
    /// dropping a `JoinHandle` does not wait for it.
    fn drop(&mut self) {
        self.cancel();
        self.handle.take();
    }
}

/// What the import dialog asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickKind {
    Folder,
    Zip,
    /// The missing-kit banner's `Locate folder…`.
    MissingKitFolder,
}

/// The import's file dialog, on its own thread. Dropped (the editor
/// closing) while the dialog is up, the thread is left to finish when the
/// user dismisses it; nothing reads its answer.
#[derive(Default)]
pub(crate) struct Picker {
    handle: Option<JoinHandle<Option<PathBuf>>>,
}

impl Picker {
    pub(crate) fn busy(&self) -> bool {
        self.handle.is_some()
    }

    /// Open the dialog unless one is already up.
    pub(crate) fn start(&mut self, kind: PickKind) -> bool {
        if self.busy() {
            return false;
        }
        let spawned = std::thread::Builder::new()
            .name("drums-import-dialog".into())
            .spawn(move || match kind {
                PickKind::Folder => rfd::FileDialog::new()
                    .set_title("Import a drum kit folder")
                    .pick_folder(),
                PickKind::Zip => rfd::FileDialog::new()
                    .set_title("Import a drum kit .zip")
                    .add_filter("Drum kit archive", &["zip"])
                    .pick_file(),
                PickKind::MissingKitFolder => rfd::FileDialog::new()
                    .set_title("Locate the missing drum kit's folder")
                    .pick_folder(),
            });
        match spawned {
            Ok(h) => {
                self.handle = Some(h);
                true
            }
            Err(e) => {
                tracing::warn!("could not open the import dialog: {e}");
                false
            }
        }
    }

    /// `Some(answer)` once the dialog closed (`answer` is `None` when it
    /// was dismissed).
    pub(crate) fn poll(&mut self) -> Option<Option<PathBuf>> {
        if !self.handle.as_ref().is_some_and(|h| h.is_finished()) {
            return None;
        }
        let h = self.handle.take()?;
        Some(h.join().ok().flatten())
    }
}
