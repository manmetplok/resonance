//! Where the app keeps per-user state: the config dir (`recent.json`,
//! `settings.json`) and the data dir (user track presets).
//!
//! A process that built its app with `Resonance::new_for_test*` is
//! *hermetic*: from then on every one of these resolves under a
//! per-process temp root instead of the machine's real `~/.config` /
//! `~/.local/share` (code review STATE-14 / FU-D5). Tests used to read the
//! real preset folder (so a golden depended on what presets the developer
//! had saved) and write the real `recent.json` (a completed save or load
//! adds to recents), which is how tempfile paths ended up in the
//! developer's recent-projects list.
//!
//! The switch is process-wide and one-way on purpose: the state it guards
//! is itself process-wide (one `recent.json`, one preset folder), and a
//! test binary never wants some of its apps on the real files.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static HERMETIC_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Make this process hermetic (idempotent) and return its temp root.
pub(crate) fn enter_hermetic() -> &'static Path {
    HERMETIC_ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("resonance-hermetic-{}", std::process::id()));
        // Best effort: every writer creates its own parent dir anyway.
        let _ = std::fs::create_dir_all(&root);
        root
    })
}

/// The per-process temp root, once the process is hermetic.
#[doc(hidden)]
pub fn hermetic_root() -> Option<&'static Path> {
    HERMETIC_ROOT.get().map(PathBuf::as_path)
}

/// `dirs::config_dir()`, or `<hermetic root>/config` in a test process.
pub fn config_dir() -> Option<PathBuf> {
    match hermetic_root() {
        Some(root) => Some(root.join("config")),
        None => dirs::config_dir(),
    }
}

/// `dirs::data_dir()`, or `<hermetic root>/data` in a test process.
pub fn data_dir() -> Option<PathBuf> {
    match hermetic_root() {
        Some(root) => Some(root.join("data")),
        None => dirs::data_dir(),
    }
}
