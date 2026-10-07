//! Data-driven per-bank quirks (doc #164).
//!
//! These reproduce — from the scanned inventory / languages / acoustic
//! config — the behaviour previously hardcoded per `VocalVoicebank` enum
//! arm in `resonance-app`'s `vocal_svs/paths.rs`. Working from the data
//! means a freshly-dropped bank gets the right treatment without a code
//! change. The answers themselves live in [`super::phonetics`]; the
//! manifest only supplies this bank's data.

use super::{ExpressionCurve, VoicebankManifest};

impl VoicebankManifest {
    /// The dict key to feed the acoustic model for a G2P ARPAbet symbol.
    /// See [`PhonemeInventory::phoneme_name`](super::PhonemeInventory::phoneme_name).
    pub fn phoneme_name(&self, ph: &str) -> String {
        self.inventory.phoneme_name(ph)
    }

    /// Per-token id for the acoustic model's `languages` input, or `None`
    /// when this bank takes no language input (`use_lang_id` is off).
    /// See [`PhonemeInventory::language_id`](super::PhonemeInventory::language_id).
    pub fn language_id(&self, ph: &str) -> Option<i64> {
        if !self.accepts_language_id {
            return None;
        }
        Some(self.inventory.language_id(ph, &self.languages))
    }

    /// Replace a G2P ARPAbet symbol the bank's dict lacks with its nearest
    /// available substitute. See
    /// [`PhonemeInventory::substitute_phoneme`](super::PhonemeInventory::substitute_phoneme).
    pub fn substitute_phoneme(&self, ph: &str) -> String {
        self.inventory.substitute_phoneme(ph)
    }

    /// Whether this bank's pipeline accepts the given expression curve.
    /// See [`CurveSupport::supports`](super::CurveSupport::supports).
    pub fn supports_curve(&self, curve: ExpressionCurve) -> bool {
        self.curve_support().supports(curve)
    }
}
