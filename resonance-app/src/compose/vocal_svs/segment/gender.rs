//! The `gender` curve: the timbre chip as a formant shift.

use resonance_music_theory::{VocalParams, VocalTimbre};
use resonance_svs::ds::SampleCurve;

use super::super::registry::BankProfile;
use super::f0::F0_TIMESTEP;

/// Gender curve maps to the acoustic model's `gender` ONNX input,
/// which shifts formants brighter / darker (range `[-1, +1]`, 0 =
/// neutral: the bank as its author voiced it). The dsconfig's
/// `use_key_shift_embed` flag is what makes the input exist; it is a
/// runtime input, not the training-time augmentation alone. Other
/// per-frame curves (`energy`, `breathiness`, `voicing`, `tension`)
/// are built elsewhere. The `timbre` chip selects a landmark on the
/// brightness axis, scaled per bank ([`BankProfile::gender_scale`]); the
/// curve is constant across the segment so the formant character stays
/// consistent.
///
/// The default chip, Warm, is neutral. It used to be `-0.15`, which cost
/// TIGER its words: Whisper heard 37 % of the audition phrase across
/// TIGER's seven modes at `-0.15` against 92 % at 0 (`vocal-rendering.md`
/// § Measurements); Lilia and Meiji did no worse at 0. Nothing darker
/// than neutral is offered any more, and TIGER (`gender_scale` 0.25)
/// gets a quarter of the swing — it held 93 % at `+0.075` and fell to
/// 71 % at `+0.15`.
pub(super) fn timbre_gender(timbre: VocalTimbre) -> f64 {
    match timbre {
        VocalTimbre::Warm => 0.0,
        VocalTimbre::Edged => 0.05,
        VocalTimbre::Airy => 0.20,
        VocalTimbre::Bright => 0.30,
    }
}

pub(super) fn build_gender_curve(params: &VocalParams, bank: &BankProfile, curve_len: usize) -> SampleCurve {
    let gender_value = timbre_gender(params.timbre) * bank.gender_scale() as f64;
    SampleCurve {
        samples: vec![gender_value; curve_len],
        timestep: F0_TIMESTEP,
    }
}
