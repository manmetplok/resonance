//! `DsSegment` builder. Orchestrates three stages, each in its own
//! submodule:
//!
//! - [`duration`] walks the clip's MIDI notes and builds the parallel
//!   phoneme / note / duration arrays (G2P, slur handling,
//!   consonant/vowel splits, AP/SP insertion).
//! - [`f0`] samples the per-frame f0 curve from those arrays and
//!   applies portamento + vibrato.
//! - [`tension`] derives the optional per-frame tension curve from
//!   the same f0 metadata for voicebanks that accept it.
//!
//! The orchestrator below stitches the stages together and assembles
//! the final `DsSegment` the pipeline consumes.
//!
//! Each syllable's text comes from `params.draft`, is run through G2P
//! to ARPAbet, and is laid out so consonants get a brisk slice and
//! the vowel(s) get the remainder. The `f0_seq` is sampled at a
//! fixed `F0_TIMESTEP` interval over the whole segment.

use resonance_audio::types::MidiNote;
use resonance_music_theory::{VocalParams, VocalTimbre};
use resonance_svs::ds::{DsSegment, SampleCurve};

mod duration;
mod f0;
mod tension;

use duration::build_phoneme_track;
use f0::{build_f0_curve, F0_TIMESTEP};
use tension::build_tension_curve;

/// Build a single `DsSegment` covering every note in the clip. See
/// the module docs for the stage breakdown.
pub(super) fn build_segment(
    notes: &[MidiNote],
    params: &VocalParams,
    lyrics: &[String],
    ticks_per_quarter: u32,
    bpm: f32,
) -> DsSegment {
    let track = build_phoneme_track(notes, params, lyrics, ticks_per_quarter, bpm);
    let f0_curve = build_f0_curve(&track, params);
    let tension = build_tension_curve(&f0_curve, params);
    let gender = build_gender_curve(params, f0_curve.samples.len());

    DsSegment {
        offset: 0.0,
        ph_seq: track.ph_seq,
        ph_dur: track.ph_dur,
        ph_num: Vec::new(),
        note_seq_midi: track.note_seq_midi,
        note_dur: track.note_dur,
        note_slur: Vec::new(),
        f0: SampleCurve {
            samples: f0_curve.samples,
            timestep: F0_TIMESTEP,
        },
        gender,
        // NOTE on `velocity`: TIGER does accept a per-frame `velocity`
        // input, but in DiffSinger semantics velocity is a *phoneme-
        // duration* multiplier (>1.0 shortens, <1.0 lengthens), not
        // the attack-strength knob it sounds like. Feeding non-1.0
        // values smeared the rendered audio down to ~-60 dB during
        // testing, so we deliberately leave it as default (the
        // pipeline fills with 1.0 internally). The per-syllable
        // velocities computed by `derive_vocal` still drive MIDI clip
        // dynamics; bridging them into the SVS model needs a different
        // parameter (and probably training-set characterisation) than
        // this knob provides.
        velocity: SampleCurve::default(),
        energy: SampleCurve::default(),
        breathiness: SampleCurve::default(),
        voicing: SampleCurve::default(),
        tension,
        languages: track.languages,
    }
}

/// Gender curve maps to the acoustic model's `gender` ONNX input,
/// which shifts formants brighter / darker (range `[-1, +1]`, 0 =
/// neutral). The dsconfig's `use_key_shift_embed` flag is unrelated —
/// that's about training-time pitch-shift augmentation, not a runtime
/// input. Other per-frame curves (`energy`, `breathiness`, `voicing`,
/// `tension`) aren't accepted by the TIGER model and are left as
/// `SampleCurve::default()`. The `timbre` chip selects a landmark on
/// the brightness axis; the curve is constant across the segment so
/// the formant character stays consistent.
///
/// Empirically-tuned band, characterised against TIGER (the tightest
/// of the three voicebanks): the negative side has a hard ceiling
/// around `-0.20`, and the positive side starts losing intelligibility
/// past about `+0.35` — whisper transcribes a `+0.50` Bright TIGER as
/// "my my my" instead of the test lyric. Lilia and Meiji are robust
/// across the band. If you widen, do it positive-side only and re-run
/// the sweep harness to confirm intelligibility doesn't collapse.
fn build_gender_curve(params: &VocalParams, curve_len: usize) -> SampleCurve {
    let gender_value = match params.timbre {
        VocalTimbre::Warm => -0.15,
        VocalTimbre::Edged => -0.05,
        VocalTimbre::Airy => 0.20,
        VocalTimbre::Bright => 0.30,
    };
    SampleCurve {
        samples: vec![gender_value; curve_len],
        timestep: F0_TIMESTEP,
    }
}
