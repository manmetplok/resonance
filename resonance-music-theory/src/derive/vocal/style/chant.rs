//! Chant: hip-hop / spoken-word monotone-leaning vocal anchored on
//! the chord root in a narrow 5-semitone band around the speaking
//! pitch. Bursts of fast syllables (sixteenth-feel by default,
//! triplet-feel on ~30 % of lines) pack into the front of each line
//! with a wider breath-gap at the end. Occasional "spit" — a 3-4
//! semitone lift on a non-edge syllable for emphasis, then snap back.

use crate::rng::XorShift;

use super::super::melody::snap_to_scale;
use super::super::params::VocalParams;
use super::super::VocalContext;
use super::{
    beat_strength, cadence_pitch, chord_tone_nearest, phrase_role, rhythm_trim, shape_velocity,
    LineState, StepInputs, VelocityShape, VocalStyleProfile,
};

pub(super) struct ChantProfile;

#[derive(Default, Clone)]
pub(super) struct ChantLine {
    /// Speaking-pitch centre, computed from `ctx.lo` / `ctx.hi`
    /// (NOT the narrowed band — match the legacy `derive_chant`
    /// math exactly or seed-anchored output drifts).
    band_centre: u8,
    /// True ~30 % of lines — switches the syllable slot from
    /// straight sixteenths to a triplet-swing pattern.
    triplet_feel: bool,
    /// Pre-triplet-jitter slot width = `sing_span / line_syl`.
    base_slot: f32,
    /// Hand-off cell for the per-syllable `spit` flag, set in
    /// `pick_pitch` and drained in `velocity`. Lives on the per-line
    /// scratchpad (not a thread-local) so the walker is `Send` and the
    /// flag's lifetime matches the line it's drawn against.
    spit_flag: std::cell::Cell<bool>,
}

impl VocalStyleProfile for ChantProfile {
    type LineExtras = ChantLine;

    fn band(&self, ctx: &VocalContext) -> (u8, u8) {
        let span = ctx.hi as i16 - ctx.lo as i16;
        let centre = (ctx.lo as i16 + (span * 4) / 10).clamp(ctx.lo as i16, ctx.hi as i16) as u8;
        let band_lo = (centre as i16 - 2).clamp(ctx.lo as i16, ctx.hi as i16) as u8;
        let band_hi = (centre as i16 + 3).clamp(ctx.lo as i16, ctx.hi as i16) as u8;
        (band_lo, band_hi)
    }

    fn init_prev_pitch(&self, ctx: &VocalContext, band: (u8, u8)) -> u8 {
        // Original chant `centre` is built from `ctx.lo`/`ctx.hi`,
        // not from the band — match that or the deterministic
        // output drifts.
        let span = ctx.hi as i16 - ctx.lo as i16;
        let centre = (ctx.lo as i16 + (span * 4) / 10).clamp(ctx.lo as i16, ctx.hi as i16) as u8;
        snap_to_scale(centre, ctx.scale, band.0, band.1)
    }

    fn breath_frac(&self, params: &VocalParams) -> f32 {
        params.breath.clamp(0.0, 0.9).max(0.18)
    }

    fn min_dur_ticks(&self, tpb: u64) -> u64 {
        (tpb / 8).max(1)
    }

    fn begin_line(
        &mut self,
        rng: &mut XorShift,
        ctx: &VocalContext,
        line: &LineState<ChantLine>,
    ) -> ChantLine {
        // Triplet-feel draw is the first rng action of the line —
        // matches original `derive_chant`.
        let triplet_feel = rng.next_f32() < 0.30;
        let base_slot = line.sing_span / line.line_syl as f32;
        // Centre uses ctx.lo / ctx.hi, not the narrowed band.
        let span = ctx.hi as i16 - ctx.lo as i16;
        let centre =
            (ctx.lo as i16 + (span * 4) / 10).clamp(ctx.lo as i16, ctx.hi as i16) as u8;
        ChantLine {
            band_centre: centre,
            triplet_feel,
            base_slot,
            spit_flag: std::cell::Cell::new(false),
        }
    }

    fn slot(&self, line: &LineState<ChantLine>, s: u32) -> f32 {
        if line.extras.triplet_feel {
            match s % 3 {
                0 => line.extras.base_slot * 1.20,
                _ => line.extras.base_slot * 0.90,
            }
        } else {
            line.extras.base_slot
        }
    }

    fn rubato_max(&self, _line: &LineState<ChantLine>, _s: u32, slot: f32) -> f32 {
        slot * 0.08
    }

    /// Recitation has its own triplet-feel rhythm engine; a half-slot
    /// anticipation would smear the spit pattern, so chant keeps its
    /// micro-rubato on every syllable.
    fn stress_syncopation_chance(&self) -> f32 {
        0.0
    }

    fn pick_pitch(
        &self,
        ctx: &VocalContext,
        line: &LineState<ChantLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
    ) -> u8 {
        // Order: punch_down draw → spit draw → spit-lift draws OR
        // the s%4 branch. Must match `derive_chant` exactly.
        let centre = line.extras.band_centre;
        let punch_down = beat_strength(inp.beat_round, ctx.beats_per_bar) >= 0.65
            && rng.next_f32() < 0.5;
        let spit = !punch_down
            && rng.next_f32() < 0.08
            && inp.s > 0
            && inp.s + 1 < line.line_syl;
        // Stash the spit flag for `velocity` to pick up so it can
        // apply the extra +0.12 lift without re-drawing the
        // predicate (which would advance rng out of sync).
        line.extras.spit_flag.set(spit);
        let pitch_pre = if inp.is_final {
            cadence_pitch(
                phrase_role(line.line_idx, ctx.line_syllables.len()),
                inp.chord,
                ctx.scale,
                inp.prev_pitch,
                (line.band_lo, line.band_hi),
            )
            .unwrap_or_else(|| {
                inp.chord
                    .and_then(|c| chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), centre))
                    .unwrap_or(centre)
            })
        } else if inp.s == 0 {
            inp.chord
                .and_then(|c| chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), centre))
                .unwrap_or(centre)
        } else if punch_down {
            inp.chord
                .and_then(|c| {
                    chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), inp.prev_pitch)
                })
                .unwrap_or(inp.prev_pitch)
        } else if spit {
            let lift = (rng.next_range(2) as i16) + 3;
            ((inp.prev_pitch as i16 + lift).clamp(line.band_lo as i16, line.band_hi as i16)) as u8
        } else if inp.s.is_multiple_of(4) && rng.next_f32() < 0.55 {
            let dir: i16 = if rng.next_f32() < 0.5 { 1 } else { -1 };
            ((inp.prev_pitch as i16 + dir).clamp(line.band_lo as i16, line.band_hi as i16)) as u8
        } else {
            inp.prev_pitch
        };

        snap_to_scale(pitch_pre, ctx.scale, line.band_lo, line.band_hi)
    }

    fn dur_beats(
        &self,
        _line: &LineState<ChantLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        inp.slot * rhythm_trim(rng, 0.85, inp.beat_round, beats_per_bar, 0.20)
    }

    fn velocity(
        &self,
        line: &LineState<ChantLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        // Chant's velocity bump (the "spit" lift) shares the same
        // boolean `pick_pitch` already drew. Re-drawing it here would
        // desync rng, so `pick_pitch` stashed the flag in the per-line
        // scratchpad; drain it here.
        let mut v = shape_velocity(
            rng,
            &VelocityShape {
                base: 0.65,
                arch: 0.4,
                accent: 1.0,
                jitter: 0.10,
            },
            inp.progress_in_line,
            inp.beat_round,
            beats_per_bar,
        );
        if line.extras.spit_flag.replace(false) {
            v = (v + 0.12).clamp(0.4, 1.0);
        }
        v
    }
}
