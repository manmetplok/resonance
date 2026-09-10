//! Where downloaded `.nam` profiles live on disk.
//!
//! Deliberately NOT under [`crate::tone3000`], even though the Tone3000
//! browser is what fills the directory: `initialize()` has to seed the
//! model browser from it on a headless build too, and everything in
//! `tone3000` is behind the `editor` feature.

/// Subdirectory under the user's data dir where downloaded models live.
pub const MODEL_SUBDIR: &str = "resonance/amp-models/tone3000";

/// Resolve the per-user directory where downloaded models are stored.
/// Returns `None` if the platform has no XDG data dir (extremely
/// unusual, but we don't want to panic on it).
pub fn models_dir() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|d| d.join(MODEL_SUBDIR))
}
