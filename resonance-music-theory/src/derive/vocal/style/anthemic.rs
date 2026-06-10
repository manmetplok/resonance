//! Anthemic: wide-range chorus melody. Each line arcs to a peak
//! roughly 60 % through the line and the syllable closest to that
//! peak (the "money note") leaps to the highest in-range chord tone.
//! Final syllables get an extra-long sustain (1.6× beat-step vs
//! 1.4× elsewhere) for the held-cadence chorus feel. Strict grid
//! (no rubato), with strong-beat chord-tone anchoring on every
//! other syllable.

use crate::rng::XorShift;

use super::super::super::motif_bass::chord_tones_in_register;
use super::super::melody::snap_to_scale;
use super::super::params::VocalParams;
use super::super::VocalContext;
use super::{
    cadence_pitch, chord_tone_nearest, phrase_role, rhythm_trim, shape_velocity, LineState,
    StepInputs, VelocityShape, VocalStyleProfile,
};

pub(super) struct AnthemicProfile;

#[derive(Default, Clone)]
pub(super) struct AnthemicLine {
    /// Index of the climax syllable (= round(line_syl * 0.6)), or
    /// `u32::MAX` when `line_syl < 4` (too short for a climax).
    climax_idx: u32,
    has_climax: bool,
}

impl VocalStyleProfile for AnthemicProfile {
    type LineExtras = AnthemicLine;

    fn init_prev_pitch(&self, ctx: &VocalContext, band: (u8, u8)) -> u8 {
        snap_to_scale(((band.0 as u16 + band.1 as u16) / 2) as u8, ctx.scale, band.0, band.1)
    }

    fn breath_frac(&self, params: &VocalParams) -> f32 {
        (params.breath.clamp(0.0, 0.9) * 0.6).max(0.10)
    }

    fn begin_line(
        &mut self,
        _rng: &mut XorShift,
        _ctx: &VocalContext,
        line: &LineState<AnthemicLine>,
    ) -> AnthemicLine {
        let climax_idx = ((line.line_syl as f32 * 0.6).round() as u32).min(line.line_syl - 1);
        AnthemicLine {
            climax_idx,
            has_climax: line.line_syl >= 4,
        }
    }

    fn pick_pitch(
        &self,
        ctx: &VocalContext,
        line: &LineState<AnthemicLine>,
        inp: &StepInputs<'_>,
        _rng: &mut XorShift,
    ) -> u8 {
        // Per-line arch: peak at t=0.6. No rng draws here.
        let t = inp.progress_in_line;
        let arch = 1.0 - ((t - 0.6).abs() / 0.6_f32.max(1.0 - 0.6_f32)).clamp(0.0, 1.0);
        let span = line.band_hi as f32 - line.band_lo as f32;
        let target = line.band_lo as f32 + (0.30 + 0.60 * arch) * span;

        let strong = inp.s.is_multiple_of(2);
        let is_climax = inp.s == line.extras.climax_idx && line.extras.has_climax;
        let candidate = target.clamp(line.band_lo as f32, line.band_hi as f32) as u8;

        if is_climax {
            inp.chord
                .and_then(|c| {
                    let tones = chord_tones_in_register(c.chord, (line.band_lo, line.band_hi));
                    tones.into_iter().max()
                })
                .unwrap_or(candidate)
        } else if inp.is_final {
            cadence_pitch(
                phrase_role(line.line_idx, ctx.line_syllables.len()),
                inp.chord,
                ctx.scale,
                inp.prev_pitch,
                (line.band_lo, line.band_hi),
            )
            .unwrap_or_else(|| {
                inp.chord
                    .and_then(|c| {
                        chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), candidate)
                    })
                    .unwrap_or(candidate)
            })
        } else if strong {
            inp.chord
                .and_then(|c| chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), candidate))
                .unwrap_or(candidate)
        } else if ctx.params.stay_in_scale {
            snap_to_scale(candidate, ctx.scale, line.band_lo, line.band_hi)
        } else {
            candidate
        }
    }

    fn dur_beats(
        &self,
        line: &LineState<AnthemicLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        let trim = 0.95 - 0.30 * line.articulation;
        let is_climax = inp.s == line.extras.climax_idx && line.extras.has_climax;
        if inp.is_final {
            // Anthemic terminal: 1.6× beat-step floor (vs the
            // shared 1.4× cap in `terminal_dur_beats`), so the
            // chorus money-note holds longer than other styles'.
            let cap = line.beat_step * 1.6;
            let normal = line.beat_step * trim;
            cap.max(normal)
        } else if is_climax {
            line.beat_step * 1.4
        } else {
            let trim_local = rhythm_trim(rng, trim, inp.beat_round, beats_per_bar, 0.20);
            line.beat_step * trim_local
        }
    }

    fn velocity(
        &self,
        line: &LineState<AnthemicLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        let mut v = shape_velocity(
            rng,
            &VelocityShape {
                base: 0.80,
                arch: 1.0,
                accent: 0.8,
                jitter: 0.07,
            },
            inp.progress_in_line,
            inp.beat_round,
            beats_per_bar,
        );
        let is_climax = inp.s == line.extras.climax_idx && line.extras.has_climax;
        if is_climax {
            v = (v + 0.10).clamp(0.4, 1.0);
        }
        v
    }
}
