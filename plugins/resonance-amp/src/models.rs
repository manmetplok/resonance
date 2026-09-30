//! Where downloaded `.nam` profiles live on disk.
//!
//! Deliberately NOT under [`crate::tone3000`], even though the Tone3000
//! browser is what fills the directory: `initialize()` has to reach the
//! model library on a headless build too, and everything in `tone3000` is
//! behind the `editor` feature.
//!
//! The library root and its layout belong to
//! `resonance_common::nam_library` (nam-model-library.md §4.1); this module
//! only names the downloads subdirectory the Tone3000 worker writes to.

/// Subdirectory under the user's data dir where downloaded models live
/// (with no `RESONANCE_AMP_MODEL_DIR` override).
pub const MODEL_SUBDIR: &str = "resonance/amp-models/tone3000";

/// Resolve the per-user directory where downloaded models are stored:
/// `<library root>/tone3000`. Returns `None` if the platform has no data
/// dir (extremely unusual, but we don't want to panic on it).
pub fn models_dir() -> Option<std::path::PathBuf> {
    resonance_common::nam_library::default_root()
        .map(|r| r.join(resonance_common::nam_library::TONE3000_DIR))
}
