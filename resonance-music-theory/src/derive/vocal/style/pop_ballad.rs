//! PopBallad: stepwise contour-driven walk with breath gaps, gentle
//! chord-tone anchoring on strong beats, surprise passing-leap on a
//! minority of weak-beat syllables, antecedent/consequent cadence on
//! each line's terminal note. The legacy default — the only style
//! that fully respects every Melody slider (contour, anchor, leap).

use crate::rng::XorShift;

use super::super::melody::{contour_height, snap_to_scale};
use super::super::VocalContext;
use super::{
    beat_strength, cadence_pitch, chord_tone_nearest, phrase_role, rhythm_trim, shape_velocity,
    terminal_dur_beats, LineState, StepInputs, VocalStyleProfile,
};

pub(super) struct PopBalladProfile;

impl VocalStyleProfile for PopBalladProfile {
    type LineExtras = ();

    fn init_prev_pitch(&self, ctx: &VocalContext, band: (u8, u8)) -> u8 {
        snap_to_scale(((band.0 as u16 + band.1 as u16) / 2) as u8, ctx.scale, band.0, band.1)
    }

    fn begin_line(
        &mut self,
        _rng: &mut XorShift,
        _ctx: &VocalContext,
        _line: &LineState<()>,
    ) {
    }

    fn rubato_max(&self, line: &LineState<()>, _s: u32, _slot: f32) -> f32 {
        line.beat_step * 0.05
    }

    fn pick_pitch(
        &self,
        ctx: &VocalContext,
        line: &LineState<()>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
    ) -> u8 {
        // Strong-beat heuristic + anchor draw — match original order.
        let strong = inp.s == 0
            || inp.s + 1 == line.line_syl
            || (inp.s.is_multiple_of(2) && beat_strength(inp.beat_round, ctx.beats_per_bar) > 0.5);
        let anchor = strong && rng.next_f32() < ctx.params.chord_tone_anchor;

        // Contour target uses GLOBAL progress across the section,
        // not per-line — Arch arches over the whole section.
        let global_idx = line.syl_cursor + inp.s;
        let global_t =
            global_idx as f32 / (ctx.total_syl.saturating_sub(1).max(1)) as f32;
        let contour_pos = contour_height(ctx.params.contour, global_t).clamp(0.0, 1.0);
        let contour_target = line.band_lo as f32
            + contour_pos * (line.band_hi as f32 - line.band_lo as f32);
        let pulled = inp.prev_pitch as f32 * 2.0 / 3.0 + contour_target / 3.0;

        // Step vs leap draws — preserve original order.
        let leap = rng.next_f32() < ctx.params.leap_range;
        let surprise_leap = !leap && rng.next_f32() < 0.12;
        let step_range = if leap {
            3..=6
        } else if surprise_leap {
            3..=4
        } else {
            1..=2
        };
        let step = (rng.next_range(*step_range.end() - *step_range.start() + 1)
            + *step_range.start()) as i16;
        let direction = if contour_target > inp.prev_pitch as f32 { 1i16 } else { -1 };
        let walked =
            (pulled as i16 + step * direction).clamp(line.band_lo as i16, line.band_hi as i16) as u8;

        if inp.is_final {
            cadence_pitch(
                phrase_role(line.line_idx),
                inp.chord,
                ctx.scale,
                inp.prev_pitch,
                (line.band_lo, line.band_hi),
            )
            .unwrap_or_else(|| {
                if anchor {
                    inp.chord
                        .and_then(|c| {
                            chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), inp.prev_pitch)
                        })
                        .unwrap_or(walked)
                } else if ctx.params.stay_in_scale {
                    snap_to_scale(walked, ctx.scale, line.band_lo, line.band_hi)
                } else {
                    walked
                }
            })
        } else if anchor {
            inp.chord
                .and_then(|c| chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), inp.prev_pitch))
                .unwrap_or(walked)
        } else if ctx.params.stay_in_scale {
            snap_to_scale(walked, ctx.scale, line.band_lo, line.band_hi)
        } else {
            walked
        }
    }

    fn dur_beats(
        &self,
        line: &LineState<()>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        let base_trim = 0.98 - 0.48 * line.articulation;
        if inp.is_final {
            terminal_dur_beats(line.beat_step, line.articulation)
        } else {
            let trim = rhythm_trim(rng, base_trim, inp.beat_round, beats_per_bar, 0.18);
            line.beat_step * trim
        }
    }

    fn velocity(
        &self,
        _line: &LineState<()>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        shape_velocity(
            rng,
            0.74,
            inp.progress_in_line,
            0.9,
            inp.beat_round,
            beats_per_bar,
            0.7,
            0.08,
        )
    }
}
