//! Where a [`super::VoicebankRegistry`] looks for banks and vocoders.

use std::path::{Path, PathBuf};

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
    /// Development models roots, scanned for the shipped layouts, highest
    /// precedence first.
    pub dev_models: Vec<PathBuf>,
}

impl VoicebankRoots {
    /// The real app's roots: the user's data dir (or the env overrides),
    /// then `$RESONANCE_SVS_MODELS_DIR`, then the source tree's
    /// `resonance-svs/models/`.
    pub fn for_machine() -> Self {
        let env_dir = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let mut dev_models: Vec<PathBuf> = Self::dev_models_from_env().into_iter().collect();
        dev_models.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resonance-svs/models"));
        let data = crate::user_dirs::data_dir().map(|d| d.join("resonance"));
        Self::from_dirs(
            env_dir(VOICEBANK_DIR_ENV),
            env_dir(VOCODER_DIR_ENV),
            data.as_deref(),
            dev_models,
        )
    }

    /// The user folders from the two overrides and the data dir. Vocoders
    /// sit beside the voicebanks (§4.2): with only the voicebank folder
    /// overridden, they go in a `vocoders/` folder next to it, so pointing
    /// `$RESONANCE_VOICEBANK_DIR` somewhere else moves both — not the
    /// banks alone, with their shared vocoders left in the data dir.
    pub fn from_dirs(
        banks: Option<PathBuf>,
        vocoders: Option<PathBuf>,
        data: Option<&Path>,
        dev_models: Vec<PathBuf>,
    ) -> Self {
        let vocoders = vocoders.or_else(|| match &banks {
            Some(banks) => Some(
                banks
                    .parent()
                    .map(|p| p.join("vocoders"))
                    .unwrap_or_else(|| PathBuf::from("vocoders")),
            ),
            None => data.map(|d| d.join("vocoders")),
        });
        Self {
            banks: banks.or_else(|| data.map(|d| d.join("voicebanks"))),
            vocoders,
            dev_models,
        }
    }

    /// A test app's roots: private folders under the process's hermetic
    /// root and no development root at all, so a test never sees the
    /// developer's banks — not the data dir's, not the source tree's
    /// `resonance-svs/models/`, and not `$RESONANCE_SVS_MODELS_DIR`'s:
    /// whether a developer downloaded the models or exported that variable
    /// must not change what a test sees. A test that wants the real banks
    /// opts in with `Resonance::test_install_dev_voicebanks_from_env`.
    pub fn hermetic() -> Self {
        let base = crate::user_dirs::hermetic_subdir("voicebank-library");
        Self {
            banks: Some(base.join("voicebanks")),
            vocoders: Some(base.join("vocoders")),
            dev_models: Vec::new(),
        }
    }

    /// `$RESONANCE_SVS_MODELS_DIR`, when it is set: the development
    /// models a developer opted into.
    pub fn dev_models_from_env() -> Option<PathBuf> {
        std::env::var_os(MODELS_DIR_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }
}
