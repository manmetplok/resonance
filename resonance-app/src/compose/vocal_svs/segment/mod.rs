//! `DsSegment` builder. Orchestrates three stages, each in its own
//! submodule:
//!
//! - [`duration`] walks the clip's MIDI notes and builds the parallel
//!   phoneme / note / duration arrays (G2P, slur handling,
//!   consonant/vowel splits, AP/SP insertion).
//! - [`f0`] samples the per-frame f0 curve from those arrays and
//!   applies portamento + vibrato.
//! - [`tension`] derives the optional per-frame tension curve from
//!   the same f0 metadata for voicebanks that accept it (from the written
//!   pitch alone when the bank's pitch model sings the unit).
//! - [`energy`] derives the velocity-driven energy curve for voicebanks
//!   that take one.
//!
//! The orchestrator below stitches the stages together and assembles
//! the final `DsSegment` the pipeline consumes.
//!
//! Each syllable's text comes from `params.draft`, is run through G2P
//! to ARPAbet, and is laid out so consonants get a brisk slice and
//! the vowel(s) get the remainder. The `f0_seq` is sampled at a
//! fixed `F0_TIMESTEP` interval over the whole segment.

use resonance_audio::types::MidiNote;
use resonance_music_theory::g2p::AssignedSyllable;
use resonance_music_theory::VocalParams;
use resonance_svs::ds::{DsSegment, SampleCurve};

mod duration;
mod energy;
mod f0;
mod gender;
mod overlay;
mod score;
mod tension;

use duration::build_phoneme_track;
use energy::build_energy_curve;
use f0::{build_f0_curve, build_step_curve, F0_TIMESTEP};
use gender::build_gender_curve;
use overlay::{curve_gain_db, overlay_envelope, overlay_pitch_cents, CurveClock};
use resonance_svs::predict::{BankModelPaths, BankScore};
use score::build_bank_score;
use tension::build_tension_curve;

use super::paths::CurveKind;
use super::registry::BankProfile;

/// Build a single `DsSegment` covering every note in `notes` from the
/// pre-resolved, voicebank-validated `assigned` syllable stream (one
/// entry per note). See the module docs for the stage breakdown and
/// [`super::resolve_clip_with_bank`] / [`super::validate_for_voicebank`]
/// for how `assigned` is produced.
///
/// The lane's expression curves (`params.curves`, §10) are layered over
/// the auto-derived result where they have points: Dynamics → energy,
/// Breathiness, Tension, Pitch bend → f0 cents offset (a bank without an
/// energy input gets Dynamics as a post-render gain instead — see
/// [`unit_curve_gain_db`]). `notes` carry section-relative ticks, which is
/// how a curve point's section beat finds its frame. `bank` spells the
/// phonemes and language ids and decides which curves reach the model.
pub fn build_segment(
    notes: &[MidiNote],
    params: &VocalParams,
    assigned: &[AssignedSyllable],
    ticks_per_quarter: u32,
    bpm: f32,
    bank: &BankProfile,
) -> DsSegment {
    build_unit_segment(notes, params, assigned, ticks_per_quarter, bpm, bank).segment
}

/// A render unit's rule-based segment plus what the bank's own models
/// need to re-time and re-pitch it (see [`build_unit_segment`]).
#[derive(Debug, Clone)]
pub struct UnitSegment {
    pub segment: DsSegment,
    /// The unit as the bank's duration / pitch / variance models see it.
    pub score: BankScore,
    /// The Pitch bend curve, cents per [`CURVE_FRAME_SEC`] frame from the
    /// segment start; empty when it has no points. Already in
    /// `segment.f0`; kept apart so it can ride on the bank's pitch.
    pub bend_cents: Vec<f64>,
    /// The bank's own models this unit renders with: the ones the lane's
    /// flags ask for that know every phoneme of [`Self::score`] (a model
    /// lacking one leaves its part to the rules for this unit — decided
    /// here, at planning, so the unit's key says which).
    pub bank_models: BankModelPaths,
}

/// [`build_segment`], keeping the [`BankScore`] and the pitch bend the
/// render needs when the bank's own models time and pitch the unit.
pub fn build_unit_segment(
    notes: &[MidiNote],
    params: &VocalParams,
    assigned: &[AssignedSyllable],
    ticks_per_quarter: u32,
    bpm: f32,
    bank: &BankProfile,
) -> UnitSegment {
    let clock = segment_clock(notes, ticks_per_quarter, bpm);
    let track = build_phoneme_track(notes, params, assigned, ticks_per_quarter, bpm, bank);
    let f0_curve = build_f0_curve(&track, params, clock);
    let n_frames = f0_curve.samples.len();

    // Dynamics → energy. Only a bank whose acoustic model takes `energy`
    // (`curve_supported`; no shipped bank does) gets it: the curve when
    // shaped, else velocity scaled by `velocity_dynamics`.
    let energy = overlay_envelope(params, CurveKind::Dynamics, bank, n_frames, clock)
        .unwrap_or_else(|| build_energy_curve(&f0_curve, params, bank));
    // Breathiness → breathiness. Same gate: no shipped bank takes it.
    let breathiness = overlay_envelope(params, CurveKind::Breathiness, bank, n_frames, clock)
        .unwrap_or_default();
    let score = build_bank_score(&track, notes, assigned);
    let bank_models = bank
        .active_bank_models(params.bank_timing, params.bank_pitch)
        .covering(score.phones.iter().map(|p| p.symbol.as_str()));
    // Tension → tension. A shaped curve on a bank that takes tension
    // replaces the auto-derived velocity/contour tension; otherwise keep
    // the baseline (which itself no-ops on TIGER). When the bank's pitch
    // model sings the unit, the contour reads the written pitch: the
    // synthesized line's portamento and vibrato are not heard, so they
    // must not lean the tension (nor move the unit's key) either.
    let tension = overlay_envelope(params, CurveKind::Tension, bank, n_frames, clock)
        .unwrap_or_else(|| {
            if bank_models.pitch.is_some() {
                build_tension_curve(&build_step_curve(&track), params, bank)
            } else {
                build_tension_curve(&f0_curve, params, bank)
            }
        });
    let gender = build_gender_curve(params, bank, n_frames);
    let bend_cents = overlay_pitch_cents(params, n_frames, clock).unwrap_or_default();

    let segment = DsSegment {
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
        energy,
        breathiness,
        voicing: SampleCurve::default(),
        tension,
        languages: track.languages,
    };
    UnitSegment {
        segment,
        score,
        bend_cents,
        bank_models,
    }
}

/// The frame → section-beat clock of a segment singing `notes`.
fn segment_clock(notes: &[MidiNote], ticks_per_quarter: u32, bpm: f32) -> CurveClock {
    let first = notes.first().map_or(0, |n| n.start_tick);
    CurveClock::new(first, ticks_per_quarter, bpm)
}

/// The post-render gain (dB per `F0_TIMESTEP` frame from the segment
/// start) the Dynamics curve asks of a segment of `n_frames` singing
/// `notes`, on a bank with no energy input; empty when there is none to
/// apply (no curve, or the bank takes energy).
pub(super) fn unit_curve_gain_db(
    notes: &[MidiNote],
    params: &VocalParams,
    ticks_per_quarter: u32,
    bpm: f32,
    bank: &BankProfile,
    n_frames: usize,
) -> Vec<f32> {
    curve_gain_db(params, bank, n_frames, segment_clock(notes, ticks_per_quarter, bpm))
}

/// Seconds per curve frame (the f0 grid).
pub(super) const CURVE_FRAME_SEC: f64 = F0_TIMESTEP;
