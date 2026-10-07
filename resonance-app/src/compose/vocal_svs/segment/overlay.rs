//! The lane's expression curves (vocal-mcp-control-and-quality.md §10)
//! sampled onto the per-frame grid the SVS segment builder emits — the
//! **same `F0_TIMESTEP` grid as the f0 curve**, so every curve lines up
//! frame for frame:
//!
//! - Dynamics → [`DsSegment::energy`](resonance_svs::ds::DsSegment::energy)
//!   on a bank with an energy input, else a post-render gain
//!   ([`curve_gain_db`], applied with the velocity gain in
//!   [`super::super::dynamics`]);
//! - Breathiness → `DsSegment::breathiness`;
//! - Tension → `DsSegment::tension`;
//! - Pitch bend → an additive cents offset folded into the f0 curve
//!   (applied in [`super::f0`], after portamento and vibrato).
//!
//! ## Time
//! Curve points are section-relative beats. The notes a segment sings
//! carry section-relative ticks, and the segment starts `SEGMENT_PAD_SEC`
//! before its first note at the section's tempo, so frame `i` sits at beat
//! `first_note_beat - pad + i * timestep` ([`CurveClock`]). Each render
//! unit builds its own clock from its own first note, so a multi-unit
//! clip samples the curve where each unit really is.
//!
//! ## When a curve applies
//! Only a curve with points. An empty curve leaves the builder's own
//! derivation alone, so a lane nobody shaped renders exactly as before.
//! A curve the bank's acoustic model does not take
//! ([`BankProfile::curve_supported`]) is a clean no-op, never an error —
//! except Dynamics, which falls back to the gain.
//!
//! ## Value mapping
//! Dynamics, Tension and Breathiness are drawn on `0..=1`; the model's
//! per-frame inputs are signed and centred on `0.0`, so `0.5` maps to
//! neutral and the ends to `±1` (the tension convention,
//! [`super::tension`]). Pitch bend maps to cents (`-50..=50`).

use resonance_music_theory::{VocalCurvePoint, VocalParams};
use resonance_svs::ds::SampleCurve;

use crate::compose::expression::{curve, real_value, sample_at_beat, NEUTRAL};

use super::super::dynamics::DYNAMICS_CURVE_MAX_DB;
use super::super::paths::CurveKind;
use super::super::registry::BankProfile;
use super::super::SEGMENT_PAD_SEC;
use super::f0::F0_TIMESTEP;

/// Maps a segment's frames to section beats.
#[derive(Debug, Clone, Copy)]
pub(in crate::compose::vocal_svs) struct CurveClock {
    /// The beat frame 0 sits at (may be negative: the leading pad of a
    /// phrase on beat 0).
    first_beat: f64,
    beats_per_frame: f64,
}

impl CurveClock {
    /// The clock of a segment whose first note starts at
    /// `first_note_tick` (section-relative) at `bpm`.
    pub(in crate::compose::vocal_svs) fn new(
        first_note_tick: u64,
        ticks_per_quarter: u32,
        bpm: f32,
    ) -> Self {
        let beats_per_sec = bpm.max(1.0) as f64 / 60.0;
        CurveClock {
            first_beat: first_note_tick as f64 / ticks_per_quarter.max(1) as f64
                - SEGMENT_PAD_SEC * beats_per_sec,
            beats_per_frame: F0_TIMESTEP * beats_per_sec,
        }
    }

    fn beat(&self, frame: usize) -> f64 {
        self.first_beat + frame as f64 * self.beats_per_frame
    }
}

/// `points` sampled at each of `n` frames (normalized), or `None` for an
/// empty curve.
fn sample_frames(points: &[VocalCurvePoint], n: usize, clock: CurveClock) -> Option<Vec<f32>> {
    if points.is_empty() || n == 0 {
        return None;
    }
    Some(
        (0..n)
            .map(|i| sample_at_beat(points, clock.beat(i)).unwrap_or(NEUTRAL))
            .collect(),
    )
}

/// Per-frame model input for `kind` (Dynamics → energy, Tension,
/// Breathiness), in the model's signed units; `None` — leaving the caller
/// on its own derivation — when the curve is empty or `bank` does not
/// take it.
pub(super) fn overlay_envelope(
    params: &VocalParams,
    kind: CurveKind,
    bank: &BankProfile,
    n_frames: usize,
    clock: CurveClock,
) -> Option<SampleCurve> {
    if !bank.curve_supported(kind) {
        return None;
    }
    let samples = sample_frames(curve(&params.curves, kind), n_frames, clock)?
        .into_iter()
        .map(envelope_to_model)
        .collect();
    Some(SampleCurve {
        samples,
        timestep: F0_TIMESTEP,
    })
}

/// Per-frame additive pitch offset in cents, or `None` for an empty
/// pitch-bend curve (f0 untouched). Every bank takes it.
pub(super) fn overlay_pitch_cents(
    params: &VocalParams,
    n_frames: usize,
    clock: CurveClock,
) -> Option<Vec<f64>> {
    let frames = sample_frames(curve(&params.curves, CurveKind::PitchBend), n_frames, clock)?;
    Some(
        frames
            .into_iter()
            .map(|v| real_value(CurveKind::PitchBend, v) as f64)
            .collect(),
    )
}

/// Per-frame post-render gain in dB from the Dynamics curve on a bank
/// with no energy input: `0.5` is 0 dB, the ends ±[`DYNAMICS_CURVE_MAX_DB`].
/// Empty when the curve is empty or the bank takes energy (it drives the
/// energy input instead).
pub(in crate::compose::vocal_svs) fn curve_gain_db(
    params: &VocalParams,
    bank: &BankProfile,
    n_frames: usize,
    clock: CurveClock,
) -> Vec<f32> {
    if bank.curve_supported(CurveKind::Dynamics) {
        return Vec::new();
    }
    sample_frames(curve(&params.curves, CurveKind::Dynamics), n_frames, clock)
        .map(|frames| {
            frames
                .into_iter()
                .map(|v| (v - NEUTRAL) * 2.0 * DYNAMICS_CURVE_MAX_DB)
                .collect()
        })
        .unwrap_or_default()
}

/// Map a normalized `0..=1` envelope value to the acoustic model's signed
/// per-frame input (`-1..=+1`, `0` neutral).
fn envelope_to_model(v: f32) -> f64 {
    (v as f64) * 2.0 - 1.0
}
