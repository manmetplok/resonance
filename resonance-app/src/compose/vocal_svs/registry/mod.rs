//! The installed voicebanks (vocal-mcp-control-and-quality.md §4.2–4.3):
//! which DiffSinger banks this machine can sing with, and what each one
//! can do.
//!
//! - [`VoicebankRegistry`] scans the voicebank folders into
//!   [`InstalledBank`]s, keyed by id. It is rescanned at startup, on every
//!   `vocal.voicebanks` call and after an install ([`VoicebankRegistry::rescan`],
//!   in [`scan`], which also defines ids, duplicate precedence and the
//!   reuse of an unchanged bank).
//! - [`BankProfile`] is what the render, the pronunciation gate and the
//!   readouts ask: one bank id's facts, from its files when it is installed
//!   and from the built-in table ([`builtin`]) when it is not.
//!
//! Where banks are found ([`VoicebankRoots`]):
//! - **User banks**: every folder of `$XDG_DATA_HOME/resonance/voicebanks/`
//!   (or `$RESONANCE_VOICEBANK_DIR`) holding an acoustic `dsconfig.yaml` at
//!   most two levels down; its id is the folder name as a slug (else the
//!   character file's name as one).
//! - **Shared vocoders** a bank names instead of bundling one:
//!   `…/resonance/vocoders/<id>/` (or `$RESONANCE_VOCODER_DIR`; with only
//!   `$RESONANCE_VOICEBANK_DIR` set, the `vocoders/` folder beside it).
//! - **Development banks**: the three shipped layouts under
//!   `$RESONANCE_SVS_MODELS_DIR` and `resonance-svs/models/`, with fixed ids
//!   (`singer/extracted` = `tiger`, `voicebanks/lilia`, `voicebanks/meiji`).
//!   A user bank with the same id wins. A test app scans none of these
//!   unless it opts in ([`VoicebankRoots::hermetic`]).
//!
//! Installing from plok.org ([`VoicebankLibrary`], §5): the index's
//! `voicebanks[]` / `vocoders[]` ([`plok_index`]), and installs that run
//! on their own threads ([`library`], `install`) into the user folders.
//! Hidden folders (an install's staging, a delete in progress) are never
//! scanned as banks.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use resonance_music_theory::VoicebankId;

pub mod builtin;
mod dict;
mod install;
mod installed;
pub mod library;
pub mod plok_index;
mod profile;
mod roots;
mod scan;
mod stamp;
mod terms;

pub use builtin::{BuiltinBank, BuiltinSpeaker, BUILTIN_BANKS};
pub use installed::InstalledBank;
pub use library::{
    InstallError, InstallErrorKind, InstallJob, InstallOutcome, InstallPhase, LibraryConfig,
    LibraryState, VoicebankLibrary,
};
pub use plok_index::{PlokIndex, PlokSidecar, PlokVocoder, PlokVoicebank};
pub use profile::{
    BankProfile, ComfortableRange, RangeSource, RenderTarget, SpeakerInfo, VoicebankMissing,
};
pub use roots::{VoicebankRoots, MODELS_DIR_ENV, VOCODER_DIR_ENV, VOICEBANK_DIR_ENV};
pub use stamp::BankStamp;
pub use terms::BankTerms;
use terms::IndexBank;

/// The scanned voicebanks. See the module docs.
#[derive(Debug, Default)]
pub struct VoicebankRegistry {
    roots: VoicebankRoots,
    banks: BTreeMap<String, Arc<InstalledBank>>,
    /// The plok.org index's banks, by id — what the typo guard accepts
    /// besides installed and built-in banks, and the hash an install
    /// record must carry to be trusted for a built-in bank's terms.
    index_banks: BTreeMap<String, IndexBank>,
    /// Bumped by every rescan (and index change), so a view can cache what
    /// it derives from the banks.
    revision: u64,
}

impl VoicebankRegistry {
    /// A registry over `roots`, scanned now.
    pub fn scan(roots: VoicebankRoots) -> Self {
        let mut registry = Self {
            roots,
            banks: BTreeMap::new(),
            index_banks: BTreeMap::new(),
            revision: 0,
        };
        registry.rescan();
        registry
    }

    pub fn roots(&self) -> &VoicebankRoots {
        &self.roots
    }

    /// Bumped by every rescan and index change.
    pub fn revision(&self) -> u64 {
        self.revision
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

    /// Where a user bank with this id would be installed; `None` for an
    /// id that is not a plain folder name.
    pub fn user_bank_dir(&self, id: &str) -> Option<PathBuf> {
        if !resonance_common::plok_download::is_safe_id(id) {
            return None;
        }
        self.roots.banks.as_ref().map(|d| d.join(id))
    }
}
