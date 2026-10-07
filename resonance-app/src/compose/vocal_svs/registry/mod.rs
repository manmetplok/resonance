//! The installed voicebanks (vocal-mcp-control-and-quality.md §4.2–4.3):
//! which DiffSinger banks this machine can sing with, and what each one
//! can do.
//!
//! - [`VoicebankRegistry`] scans the voicebank folders into
//!   [`InstalledBank`]s, keyed by id. It is rescanned at startup, on every
//!   `vocal.voicebanks` call and after an install ([`VoicebankRegistry::rescan`]).
//! - [`BankProfile`] is what the render, the pronunciation gate and the
//!   readouts ask: one bank id's facts, from its files when it is installed
//!   and from the built-in table ([`builtin`]) when it is not.
//!
//! Where banks are found ([`VoicebankRoots`]):
//! - **User banks**: every folder of `$XDG_DATA_HOME/resonance/voicebanks/`
//!   (or `$RESONANCE_VOICEBANK_DIR`) holding an acoustic `dsconfig.yaml` at
//!   most two levels down; its id is the folder name as a slug.
//! - **Shared vocoders** a bank names instead of bundling one:
//!   `…/resonance/vocoders/<id>/` (or `$RESONANCE_VOCODER_DIR`).
//! - **Development banks**: the three shipped layouts under
//!   `$RESONANCE_SVS_MODELS_DIR` and `resonance-svs/models/`, with fixed ids
//!   (`singer/extracted` = `tiger`, `voicebanks/lilia`, `voicebanks/meiji`).
//!   A user bank with the same id wins.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use resonance_music_theory::VoicebankId;
use resonance_svs::voicebank::{self, layout};

pub mod builtin;
mod dict;
mod installed;
mod profile;
mod roots;

pub use builtin::{BuiltinBank, BuiltinSpeaker, BUILTIN_BANKS};
pub use installed::InstalledBank;
pub use profile::{
    BankProfile, ComfortableRange, RangeSource, RenderTarget, SpeakerInfo, VoicebankMissing,
};
pub use roots::{VoicebankRoots, MODELS_DIR_ENV, VOCODER_DIR_ENV, VOICEBANK_DIR_ENV};

/// The scanned voicebanks. See the module docs.
#[derive(Debug, Default)]
pub struct VoicebankRegistry {
    roots: VoicebankRoots,
    banks: BTreeMap<String, Arc<InstalledBank>>,
}

impl VoicebankRegistry {
    /// A registry over `roots`, scanned now.
    pub fn scan(roots: VoicebankRoots) -> Self {
        let mut registry = Self {
            roots,
            banks: BTreeMap::new(),
        };
        registry.rescan();
        registry
    }

    /// Re-read every root: a bank installed, replaced or deleted since
    /// the last scan shows up (or goes) now.
    pub fn rescan(&mut self) {
        let mut banks = BTreeMap::new();
        if let Some(dir) = &self.roots.banks {
            for sub in subdirs(dir) {
                let Some(name) = sub.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                    continue;
                };
                let id = layout::slugify(&name);
                if id.is_empty() || banks.contains_key(&id) || !layout::looks_like_bank(&sub) {
                    continue;
                }
                if let Some(bank) = self.load(&sub, id.clone(), true) {
                    banks.insert(id, Arc::new(bank));
                }
            }
        }
        for models in &self.roots.dev_models {
            for b in BUILTIN_BANKS {
                let dir = models.join(b.dev_dir);
                if banks.contains_key(b.id) || !dir.is_dir() {
                    continue;
                }
                if let Some(bank) = self.load(&dir, b.id.to_owned(), false) {
                    banks.insert(b.id.to_owned(), Arc::new(bank));
                }
            }
        }
        self.banks = banks;
    }

    fn load(&self, dir: &Path, id: String, user: bool) -> Option<InstalledBank> {
        let manifest = match voicebank::scan(dir) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("voicebank {id} at {}: {e:#}", dir.display());
                return None;
            }
        };
        let builtin = builtin::builtin(&id);
        let declared = Some(manifest.details.vocoder_name.clone()).filter(|v| !v.is_empty());
        let vocoder_config = manifest
            .vocoder_config
            .clone()
            .or_else(|| self.library_vocoder(declared.as_deref()?));
        // Report the vocoder by its own name when it is installed (the
        // config may only name the folder it sits in, `dsvocoder`).
        let installed_name = vocoder_config
            .as_deref()
            .and_then(layout::installed_vocoder_name);
        let vocoder_config = vocoder_config.filter(|_| installed_name.is_some());
        let vocoder_name = installed_name.or(declared);
        let speakers = profile::offered_speakers(builtin, &manifest.singers);
        let name = builtin
            .map(|b| b.name.to_owned())
            .or_else(|| manifest.details.name.clone())
            .unwrap_or_else(|| manifest.display_name.clone());
        Some(InstalledBank {
            id,
            name,
            version: manifest.details.version.clone(),
            root: manifest.root.clone(),
            user,
            acoustic_config: manifest.acoustic_config.clone(),
            vocoder_name,
            vocoder_config,
            speakers,
            curves: manifest.curve_support(),
            accepts_language_id: manifest.accepts_language_id,
            languages: manifest.languages.clone(),
            inventory: manifest.inventory().clone(),
            english_dict: manifest.details.english_dict.clone(),
            builtin,
            dictionary: Default::default(),
        })
    }

    /// A shared vocoder by id: the user vocoder folder, then each
    /// development root's `vocoder/` folder.
    fn library_vocoder(&self, name: &str) -> Option<PathBuf> {
        self.roots
            .vocoders
            .iter()
            .cloned()
            .chain(self.roots.dev_models.iter().map(|m| m.join("vocoder")))
            .find_map(|lib| layout::find_library_vocoder(&lib, name))
    }

    pub fn roots(&self) -> &VoicebankRoots {
        &self.roots
    }

    /// Every installed bank, by id.
    pub fn installed(&self) -> impl Iterator<Item = &Arc<InstalledBank>> {
        self.banks.values()
    }

    pub fn get(&self, id: &str) -> Option<&Arc<InstalledBank>> {
        self.banks.get(id)
    }

    /// Whether `id` names a bank this app can talk about: installed, or
    /// one of the built-in banks (which an install can provide).
    pub fn is_known(&self, id: &str) -> bool {
        self.banks.contains_key(id) || builtin::builtin(id).is_some()
    }

    /// The facts for bank `id`, installed or not.
    pub fn profile(&self, id: &VoicebankId) -> BankProfile {
        BankProfile::new(id.clone(), self.banks.get(id.as_str()).cloned())
    }

    /// Where a user bank with this id would be installed.
    pub fn user_bank_dir(&self, id: &str) -> Option<PathBuf> {
        self.roots.banks.as_ref().map(|d| d.join(id))
    }
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}
