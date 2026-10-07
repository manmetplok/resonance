//! Where a [`super::VoicebankRegistry`] looks for banks and vocoders.

use std::path::PathBuf;

/// User voicebank folder override.
pub const VOICEBANK_DIR_ENV: &str = "RESONANCE_VOICEBANK_DIR";
/// Shared vocoder folder override.
pub const VOCODER_DIR_ENV: &str = "RESONANCE_VOCODER_DIR";
/// Development models root (the three shipped layouts).
pub const MODELS_DIR_ENV: &str = "RESONANCE_SVS_MODELS_DIR";

/// Where a registry looks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VoicebankRoots {
    /// The user voicebank folder: one bank per subfolder.
    pub banks: Option<PathBuf>,
    /// The user vocoder folder: one vocoder per subfolder.
    pub vocoders: Option<PathBuf>,
    /// Development models roots, scanned for the shipped layouts.
    pub dev_models: Vec<PathBuf>,
}

impl VoicebankRoots {
    /// The real app's roots: the user's data dir (or the env overrides),
    /// then `$RESONANCE_SVS_MODELS_DIR`, then the source tree's
    /// `resonance-svs/models/`.
    pub fn for_machine() -> Self {
        let data = crate::user_dirs::data_dir().map(|d| d.join("resonance"));
        let env_dir = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let mut dev_models: Vec<PathBuf> = env_dir(MODELS_DIR_ENV).into_iter().collect();
        dev_models.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resonance-svs/models"));
        Self {
            banks: env_dir(VOICEBANK_DIR_ENV)
                .or_else(|| data.as_ref().map(|d| d.join("voicebanks"))),
            vocoders: env_dir(VOCODER_DIR_ENV).or_else(|| data.map(|d| d.join("vocoders"))),
            dev_models,
        }
    }

    /// A test app's roots: private folders under the process's hermetic
    /// root, so a test never sees the developer's banks, plus
    /// `$RESONANCE_SVS_MODELS_DIR` when it is set — the explicit opt-in to
    /// test against real banks. The source tree's `resonance-svs/models/`
    /// is left out: whether a developer downloaded the models must not
    /// change what a test sees.
    pub fn hermetic() -> Self {
        let base = crate::user_dirs::hermetic_subdir("voicebank-library");
        Self {
            banks: Some(base.join("voicebanks")),
            vocoders: Some(base.join("vocoders")),
            dev_models: std::env::var_os(MODELS_DIR_ENV)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .into_iter()
                .collect(),
        }
    }
}
