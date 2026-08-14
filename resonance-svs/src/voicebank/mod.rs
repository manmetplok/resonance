//! Voicebank manifest: scan one DiffSinger voicebank folder, auto-detect
//! its on-disk layout, and validate it actually loads.
//!
//! A *voicebank* is a folder shipping an acoustic `dsconfig.yaml`, a
//! vocoder config, a phoneme dictionary, the referenced ONNX models, and
//! — for multi-speaker banks — a `speakers` list inside the acoustic
//! config.
//!
//! The module is split by reason-to-change:
//!
//! * [`layout`] — where the files sit. Path probing plus cheap metadata
//!   reads; the only part touching `std::fs`. Changes when a vendor ships
//!   a new folder layout.
//! * [`phonetics`] — what the symbols mean. The ARPAbet substitution
//!   table, the alphabet heuristic, dict-key resolution and the expression
//!   -curve capability answers, all pure functions of an in-memory
//!   inventory. Changes when the G2P alphabet or curve set changes.
//! * [`VoicebankManifest`] — the thin descriptor joining the two, and the
//!   validation entry point that re-exercises the [`crate::config`]
//!   loaders so an unusable bank is rejected with a descriptive error
//!   before the pipeline ever touches an ONNX session.

pub mod layout;
pub mod phonetics;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

pub use layout::BankLayout;
pub use phonetics::{
    detect_phoneme_target, nearest_substitutes, CurveSupport, ExpressionCurve, PhonemeInventory,
    PhonemeTarget,
};

/// One selectable speaker inside a multi-speaker voicebank. Single-speaker
/// banks carry no singers (the manifest's `singers` is empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingerInfo {
    /// The identifier the pipeline passes as `speaker` — the raw name from
    /// the acoustic config's `speakers` list.
    pub id: String,
    /// Human-readable label. Currently identical to `id`; kept separate so
    /// a future prettifier (or a `character.yaml` lookup) can diverge it
    /// without changing the selection key.
    pub display_name: String,
}

/// A scanned, resolved description of one voicebank folder.
///
/// Every path is absolute (resolved against the bank root). Optional model
/// configs are `None` when the bank doesn't ship that stage. The manifest
/// is data-only: nothing here opens an ONNX session — see [`Self::validate`]
/// for the load gate.
#[derive(Debug, Clone)]
pub struct VoicebankManifest {
    /// Normalized slug derived from the folder name (lowercase, ASCII
    /// alphanumerics, single `-` between runs). Stable id for lookups.
    pub id: String,
    /// The folder name as shipped, for display.
    pub display_name: String,
    /// Absolute path to the bank root.
    pub root: PathBuf,

    /// Acoustic `dsconfig.yaml` (always present in a usable bank).
    pub acoustic_config: PathBuf,
    /// Vocoder config (`vocoder.yaml`), if one was found.
    pub vocoder_config: Option<PathBuf>,
    /// Phoneme dictionary the acoustic config points at.
    pub phoneme_dict: PathBuf,

    /// Resolved variance / duration / linguistic / pitch model paths from
    /// the acoustic config, when the bank declares them.
    pub variance_model: Option<PathBuf>,
    pub dur_model: Option<PathBuf>,
    pub linguistic_model: Option<PathBuf>,
    pub pitch_model: Option<PathBuf>,

    /// Selectable speakers. Empty for a single-speaker bank.
    pub singers: Vec<SingerInfo>,
    /// Phoneme inventory in token-id order (index = id for array/`.txt`
    /// dicts; sorted by explicit id for object dicts).
    pub phonemes: Vec<String>,
    /// `languages.json` contents (name -> language id), empty if absent.
    pub languages: BTreeMap<String, i64>,

    /// Alphabet the dict is written in, detected from the inventory.
    pub phoneme_target: PhonemeTarget,
    /// Acoustic model accepts a per-token `languages` input (multi-language
    /// bank). Gates [`Self::language_id`].
    pub accepts_language_id: bool,
    /// Acoustic model accepts an `energy` curve input — drives the
    /// Dynamics expression curve.
    pub accepts_energy: bool,
    /// Acoustic model accepts a `breathiness` curve input.
    pub accepts_breathiness: bool,
    /// Acoustic model accepts a `tension` curve input.
    pub accepts_tension: bool,
    /// Acoustic model accepts a `voicing` curve input.
    pub accepts_voicing: bool,

    /// The phonetic domain model built from [`Self::phonemes`] at scan
    /// time: an O(1) membership index plus every per-token answer
    /// ([`Self::phoneme_name`], [`Self::language_id`],
    /// [`Self::substitute_phoneme`]), so those stay cheap on the render
    /// path and testable without a bank on disk.
    inventory: PhonemeInventory,
}

/// Scan a voicebank folder into a [`VoicebankManifest`].
///
/// Probes the on-disk layout (see [`layout::probe`]) and joins the result
/// with the phonetic model derived from the bank's inventory. Errors if
/// `dir` is not a directory, if no acoustic `dsconfig.yaml` can be located,
/// or if that config fails to parse / resolve its phoneme dict. Model ONNX
/// files are *not* required to exist at scan time (they are large and may
/// be absent in a config-only fixture); their existence is the pipeline's
/// concern, while [`VoicebankManifest::validate`] gates on the configs
/// loading.
pub fn scan(dir: &Path) -> Result<VoicebankManifest> {
    Ok(VoicebankManifest::from_layout(layout::probe(dir)?))
}

impl VoicebankManifest {
    /// Join a probed on-disk layout with the phonetic model its inventory
    /// implies.
    fn from_layout(layout: BankLayout) -> Self {
        let inventory = PhonemeInventory::new(layout.phonemes.iter().cloned());
        let phoneme_target = inventory.target();
        let BankLayout {
            id,
            display_name,
            root,
            acoustic_config,
            vocoder_config,
            phoneme_dict,
            variance_model,
            dur_model,
            linguistic_model,
            pitch_model,
            singers,
            phonemes,
            languages,
            accepts_language_id,
            accepts_energy,
            accepts_breathiness,
            accepts_tension,
            accepts_voicing,
        } = layout;

        VoicebankManifest {
            id,
            display_name,
            root,
            acoustic_config,
            vocoder_config,
            phoneme_dict,
            variance_model,
            dur_model,
            linguistic_model,
            pitch_model,
            singers,
            phonemes,
            languages,
            phoneme_target,
            accepts_language_id,
            accepts_energy,
            accepts_breathiness,
            accepts_tension,
            accepts_voicing,
            inventory,
        }
    }

    /// True for a single-speaker bank (no `speakers` declared).
    pub fn is_single_speaker(&self) -> bool {
        self.singers.is_empty()
    }

    /// The bank's phonetic model: the inventory index behind
    /// [`Self::phoneme_name`] and friends.
    pub fn inventory(&self) -> &PhonemeInventory {
        &self.inventory
    }

    /// Which expression curves this bank's acoustic model can take.
    pub fn curve_support(&self) -> CurveSupport {
        CurveSupport {
            energy: self.accepts_energy,
            tension: self.accepts_tension,
            breathiness: self.accepts_breathiness,
        }
    }

    /// Confirm the bank is loadable by exercising every config loader the
    /// pipeline depends on: the acoustic config, the vocoder config (when
    /// present), and the phoneme dictionary. Returns the first descriptive
    /// error, or `Ok(())` when the bank parses cleanly.
    ///
    /// This re-reads the configs rather than trusting the scan so a bank
    /// edited or truncated after scanning is still caught.
    pub fn validate(&self) -> Result<()> {
        layout::validate_configs(
            &self.id,
            &self.acoustic_config,
            &self.phoneme_dict,
            self.vocoder_config.as_deref(),
        )
    }

    // -----------------------------------------------------------------
    // Data-driven per-bank quirks (doc #164)
    //
    // These reproduce — from the scanned inventory / languages / acoustic
    // config — the behaviour previously hardcoded per `VocalVoicebank`
    // enum arm in `resonance-app`'s `vocal_svs/paths.rs`. Working from the
    // data means a freshly-dropped bank gets the right treatment without a
    // code change. The answers themselves live in `phonetics`; the
    // manifest only supplies this bank's data.
    // -----------------------------------------------------------------

    /// The dict key to feed the acoustic model for a G2P ARPAbet symbol.
    /// See [`PhonemeInventory::phoneme_name`].
    pub fn phoneme_name(&self, ph: &str) -> String {
        self.inventory.phoneme_name(ph)
    }

    /// Per-token id for the acoustic model's `languages` input, or `None`
    /// when this bank takes no language input (`use_lang_id` is off).
    /// See [`PhonemeInventory::language_id`].
    pub fn language_id(&self, ph: &str) -> Option<i64> {
        if !self.accepts_language_id {
            return None;
        }
        Some(self.inventory.language_id(ph, &self.languages))
    }

    /// Replace a G2P ARPAbet symbol the bank's dict lacks with its nearest
    /// available substitute. See [`PhonemeInventory::substitute_phoneme`].
    pub fn substitute_phoneme(&self, ph: &str) -> String {
        self.inventory.substitute_phoneme(ph)
    }

    /// Whether this bank's pipeline accepts the given expression curve.
    /// See [`CurveSupport::supports`].
    pub fn supports_curve(&self, curve: ExpressionCurve) -> bool {
        self.curve_support().supports(curve)
    }
}
