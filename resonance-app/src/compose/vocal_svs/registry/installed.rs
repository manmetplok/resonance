//! One scanned bank: its files and the facts read from them.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use resonance_music_theory::g2p::PhonemeDictionary;
use resonance_svs::voicebank::{CurveSupport, PhonemeInventory};

use super::{dict, BankStamp, BuiltinBank, SpeakerInfo};

/// One installed voicebank, as scanned.
#[derive(Debug)]
pub struct InstalledBank {
    pub id: String,
    /// The built-in display name, else the character file's, else the
    /// folder name.
    pub name: String,
    pub version: Option<String>,
    /// The bank folder.
    pub root: PathBuf,
    /// The folder the scan found the bank in (`root` as listed, before
    /// any normalisation) — what a rescan matches it by.
    pub scanned_dir: PathBuf,
    /// The files the scan read, as they were; a rescan keeps this bank
    /// while they are unchanged.
    pub stamp: BankStamp,
    /// Whether the bank came from the user folder or a development root.
    pub user: bool,
    pub acoustic_config: PathBuf,
    /// The vocoder the acoustic config asks for (`dsvocoder`, or a shared
    /// vocoder's id); `None` when it names none.
    pub vocoder_name: Option<String>,
    /// The vocoder config the render uses: the bank's own, else the
    /// shared one named by `vocoder_name`. `None` = vocoder missing.
    pub vocoder_config: Option<PathBuf>,
    /// The speakers offered (the built-in allow-list when there is one).
    pub speakers: Vec<SpeakerInfo>,
    pub curves: CurveSupport,
    pub accepts_language_id: bool,
    pub languages: BTreeMap<String, i64>,
    pub inventory: PhonemeInventory,
    /// The bank's own duration / pitch / variance models (`dsdur/`,
    /// `dspitch/`, `dsvariance/` next to the acoustic config), where it
    /// ships them.
    pub models: resonance_svs::predict::BankModelPaths,
    /// The bank's `dsdict-en.yaml`, if it ships one.
    pub english_dict: Option<PathBuf>,
    pub builtin: Option<&'static BuiltinBank>,
    /// What a plok.org install recorded (version, hash, terms, credit);
    /// `None` for a bank dropped in by hand or a development bank.
    pub plok: Option<super::PlokSidecar>,
    /// The English dictionary, read on first use (TIGER's has ~11k
    /// entries; most lanes never need it parsed twice).
    pub(super) dictionary: OnceLock<Option<Arc<PhonemeDictionary>>>,
}

impl InstalledBank {
    /// The bank's English dictionary mapped to ARPAbet, or `None` when it
    /// ships none (or it cannot be read).
    pub fn dictionary(&self) -> Option<Arc<PhonemeDictionary>> {
        self.dictionary
            .get_or_init(|| {
                let path = self.english_dict.as_deref()?;
                let prefix = self.namespaces_english().then_some("en/");
                match dict::load_english_dictionary(path, prefix, &self.inventory) {
                    Ok(map) => Some(Arc::new(map)),
                    Err(e) => {
                        tracing::warn!("voicebank {}: {e}", self.id);
                        None
                    }
                }
            })
            .clone()
    }

    /// Whether the bank spells English phonemes `en/…` (Meiji).
    pub fn namespaces_english(&self) -> bool {
        resonance_music_theory::g2p::ARPABET_PHONEMES
            .iter()
            .any(|ph| {
                self.inventory
                    .dict_key_for(ph)
                    .is_some_and(|k| k.starts_with("en/"))
            })
    }
}
