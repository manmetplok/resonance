//! The user's NAM model and drum-kit libraries, opened once and cached on
//! `ControlEndpointState` for the `amp_models.*` / `drum_kits.*` handlers
//! (nam-model-library.md §9.3, drums-plugin-rework.md §8). The handlers
//! are `update::control::{amp_models, drum_kits}`; the caches live here
//! because state owns the types it holds (ARCH2-05).
//!
//! The roots come from the app: the user's data dir in the real app, a
//! private temporary one in every `new_for_test*` app. A library is
//! opened (and scanned) once and kept; a request re-reads the index only
//! when it changed on disk (one `stat`) and answers from it, while the
//! rescan that notices a moved file runs off the update loop
//! ([`OffThreadRescan`]).

use std::path::PathBuf;

use resonance_common::library_marks::SharedMarks;
use resonance_common::{drumkit_library, nam_library};

/// Where the app's `amp_models.*` handlers find the model library and the
/// marks store.
#[derive(Debug, Clone, Default)]
pub struct AmpLibraryRoots {
    pub models: Option<PathBuf>,
    pub marks: Option<PathBuf>,
}

/// A library rescan kept off the update loop (code review STATE2-08), as
/// `plugins.rescan` is: a rescan walks every file under the root and
/// hashes the new ones, which must not stall the GUI. At most one runs at
/// a time; it writes the library's index (`library.json`, under the
/// library's own file lock), and the next request's `reload_if_changed`
/// — one `stat` — picks the result up. Shared by `drum_kits.*`.
#[derive(Default)]
pub(crate) struct OffThreadRescan {
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl OffThreadRescan {
    /// Run `rescan` on a thread unless one is still running.
    pub(crate) fn start(&self, name: &str, rescan: impl FnOnce() + Send + 'static) {
        use std::sync::atomic::Ordering;
        if self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        /// Clears the flag however the rescan ends, a panic included.
        struct Done(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Done {
            fn drop(&mut self) {
                self.0.store(false, std::sync::atomic::Ordering::Release);
            }
        }
        let done = Done(self.running.clone());
        let spawned = std::thread::Builder::new()
            .name(format!("{name}-rescan"))
            .spawn(move || {
                let _done = done;
                rescan();
            });
        if let Err(e) = spawned {
            tracing::warn!("{name}: could not start a library rescan: {e}");
            self.running.store(false, Ordering::Release);
        }
    }
}

/// The opened library and marks, cached across requests.
#[derive(Default)]
pub struct AmpLibraryCache {
    pub roots: AmpLibraryRoots,
    library: Option<nam_library::Library>,
    marks: Option<SharedMarks>,
    rescan: OffThreadRescan,
}

impl AmpLibraryCache {
    pub fn new(roots: AmpLibraryRoots) -> Self {
        Self {
            roots,
            ..Self::default()
        }
    }

    /// The library as last indexed. Opened and scanned on first use, so
    /// the first answer sees what is installed; after that a request
    /// re-reads `library.json` when it changed (one `stat`) and starts a
    /// rescan off the update loop ([`OffThreadRescan`]), so a model file
    /// added since shows up in a later answer.
    pub fn library(&mut self) -> &nam_library::Library {
        let roots = &self.roots;
        let first = self.library.is_none();
        let lib = self.library.get_or_insert_with(|| match &roots.models {
            Some(r) => nam_library::Library::open(r),
            None => nam_library::Library::empty(),
        });
        if let Some(root) = lib.root().map(std::path::Path::to_path_buf) {
            if first {
                if let Err(e) = lib.rescan() {
                    tracing::warn!("amp_models: library rescan failed: {e}");
                }
            } else {
                lib.reload_if_changed();
                self.rescan.start("amp-models", move || {
                    if let Err(e) = nam_library::Library::open(root).rescan() {
                        tracing::warn!("amp_models: library rescan failed: {e}");
                    }
                });
            }
        }
        lib
    }

    /// The library rescanned here and now, for a lookup that missed the
    /// last index (a model added a moment ago): the rare path that waits.
    pub fn library_rescanned(&mut self) -> &nam_library::Library {
        if self.library.is_none() {
            // The first open scans already.
            return self.library();
        }
        let lib = self.library.as_mut().expect("checked above");
        if lib.root().is_some() {
            if let Err(e) = lib.rescan() {
                tracing::warn!("amp_models: library rescan failed: {e}");
            }
        }
        lib
    }

    /// The marks store (picking up other processes' writes).
    pub fn marks(&mut self) -> &SharedMarks {
        let roots = &self.roots;
        let marks = self.marks.get_or_insert_with(|| match &roots.marks {
            Some(d) => SharedMarks::open(d).unwrap_or_else(|e| {
                tracing::warn!("amp_models: marks unavailable: {e}");
                SharedMarks::detached()
            }),
            None => SharedMarks::detached(),
        });
        marks.refresh();
        marks
    }
}

/// Where the app's `drum_kits.*` handlers find the kit library and the
/// marks store.
#[derive(Debug, Clone, Default)]
pub struct DrumKitLibraryRoots {
    pub kits: Option<PathBuf>,
    pub marks: Option<PathBuf>,
}

/// The opened library and marks, cached across requests.
#[derive(Default)]
pub struct DrumKitLibraryCache {
    pub roots: DrumKitLibraryRoots,
    library: Option<drumkit_library::Library>,
    marks: Option<SharedMarks>,
    rescan: OffThreadRescan,
}

impl DrumKitLibraryCache {
    pub fn new(roots: DrumKitLibraryRoots) -> Self {
        Self {
            roots,
            ..Self::default()
        }
    }

    /// The library as last indexed. Opened and scanned on first use, so
    /// the first answer sees what is installed; after that a request
    /// re-reads `library.json` when it changed (one `stat`) and starts a
    /// rescan off the update loop (code review STATE2-08; see
    /// [`OffThreadRescan`]), so a kit added since shows up in a
    /// later answer.
    pub fn library(&mut self) -> &drumkit_library::Library {
        let roots = &self.roots;
        let first = self.library.is_none();
        let lib = self.library.get_or_insert_with(|| match &roots.kits {
            Some(r) => drumkit_library::Library::open(r),
            None => drumkit_library::Library::empty(),
        });
        if let Some(root) = lib.root().map(std::path::Path::to_path_buf) {
            if first {
                if let Err(e) = lib.rescan() {
                    tracing::warn!("drum_kits: library rescan failed: {e}");
                }
            } else {
                lib.reload_if_changed();
                self.rescan.start("drum-kits", move || {
                    if let Err(e) = drumkit_library::Library::open(root).rescan() {
                        tracing::warn!("drum_kits: library rescan failed: {e}");
                    }
                });
            }
        }
        lib
    }

    /// The marks store (picking up other processes' writes).
    pub fn marks(&mut self) -> &SharedMarks {
        let roots = &self.roots;
        let marks = self.marks.get_or_insert_with(|| match &roots.marks {
            Some(d) => SharedMarks::open(d).unwrap_or_else(|e| {
                tracing::warn!("drum_kits: marks unavailable: {e}");
                SharedMarks::detached()
            }),
            None => SharedMarks::detached(),
        });
        marks.refresh();
        marks
    }
}

