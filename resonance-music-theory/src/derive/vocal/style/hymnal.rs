//! Hymnal: strict syllable-per-quarter rhythm with stepwise motion
//! only, narrowed to a 9-semitone band on top of `ctx.lo`, every
//! line cadencing on a chord tone of the active chord. Minimal
//! randomness — same seed gives a near-deterministic shape. Skips
//! the random phrase-start offset (strict timing is core to the
//! style) and skips the breath-gap fraction (every syllable
//! occupies its full grid slot).

use crate::rng::XorShift;

use super::super::super::motif_bass::chord_tones_in_register;
use super::super::melody::snap_to_scale;
use super::super::params::VocalParams;
use super::super::VocalContext;
use super::{
    cadence_pitch, phrase_role, shape_velocity, LineState, StepInputs, VocalStyleProfile,
};

pub(super) struct HymnalProfile;

impl VocalStyleProfile for HymnalProfile {
    type LineExtras = ();

    fn band(&self, ctx: &VocalContext) -> (u8, u8) {
        let band_hi = (ctx.lo as i16 + 9).min(ctx.hi as i16) as u8;
        (ctx.lo, band_hi)
    }

    fn init_prev_pitch(&self, ctx: &VocalContext, band: (u8, u8)) -> u8 {
        snap_to_scale(((band.0 as u16 + band.1 as u16) / 2) as u8, ctx.scale, band.0, band.1)
    }

    fn breath_frac(&self, _params: &VocalParams) -> f32 {
        0.0
    }

    fn use_phrase_start_offset(&self) -> bool {
        false
    }

    fn begin_line(&mut self, _rng: &mut XorShift, _ctx: &VocalContext, _line: &LineState<()>) {}

    fn pick_pitch(
        &self,
        ctx: &VocalContext,
        line: &LineState<()>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
    ) -> u8 {
        let centre = (line.band_lo as i16 + line.band_hi as i16) / 2;
        let drift = (centre - inp.prev_pitch as i16).signum();
        // Stepwise walk with centre-bias — order of draws matches
        // the original. Note the nested `else if rng.next_f32()` is
        // *not* short-circuited by an earlier success: each branch
        // is reached only if its predecessor failed.
        let raw_step: i16 = if rng.next_f32() < 0.12 {
            0
        } else if rng.next_f32() < 0.6 {
            drift
        } else if rng.next_f32() < 0.5 {
            1
        } else {
            -1
        };
        let mut candidate = ((inp.prev_pitch as i16 + raw_step)
            .clamp(line.band_lo as i16, line.band_hi as i16)) as u8;

        if inp.is_final {
            if let Some(picked) = cadence_pitch(
                phrase_role(line.line_idx),
                inp.chord,
                ctx.scale,
                inp.prev_pitch,
                (line.band_lo, line.band_hi),
            ) {
                candidate = picked;
            } else if let Some(c) = inp.chord {
                let tones = chord_tones_in_register(c.chord, (line.band_lo, line.band_hi));
                if let Some(picked) = tones
                    .iter()
                    .copied()
                    .min_by_key(|t| (*t as i16 - inp.prev_pitch as i16).abs())
                {
                    candidate = picked;
                }
            }
        }

        snap_to_scale(candidate, ctx.scale, line.band_lo, line.band_hi)
    }

    fn dur_beats(
        &self,
        line: &LineState<()>,
        _inp: &StepInputs<'_>,
        _rng: &mut XorShift,
        _beats_per_bar: u32,
    ) -> f32 {
        let trim = 0.92 - 0.30 * line.articulation;
        line.beat_step * trim
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
            0.72,
            inp.progress_in_line,
            0.45,
            inp.beat_round,
            beats_per_bar,
            0.4,
            0.05,
        )
    }
}
