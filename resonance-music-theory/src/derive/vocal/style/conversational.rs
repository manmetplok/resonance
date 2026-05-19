//! Conversational: talky/spoken-feel anchored on a speaking pitch a
//! hair below the centre of the range. Pitches repeat ~55 % of the
//! time, walk by 1–2 semitones otherwise, with line-edge inflection
//! (rise on syllable 0, fall on the terminal). Larger rubato than
//! PopBallad — words push and pull against the click. Ignores
//! `params.contour` because the shape comes from the per-line
//! inflection, not a section-spanning curve.

use crate::rng::XorShift;

use super::super::melody::snap_to_scale;
use super::super::VocalContext;
use super::{
    beat_strength, cadence_pitch, chord_tone_nearest, phrase_role, rhythm_trim, shape_velocity,
    terminal_dur_beats, LineState, StepInputs, VelocityShape, VocalStyleProfile,
};

pub(super) struct ConversationalProfile;

/// Speaking pitch helper: a hair below the band centre, scale-snapped.
fn conversational_speaking_pitch(ctx: &VocalContext, band: (u8, u8)) -> u8 {
    let span = band.1 as i16 - band.0 as i16;
    snap_to_scale(
        (band.0 as i16 + (span * 4) / 10).clamp(band.0 as i16, band.1 as i16) as u8,
        ctx.scale,
        band.0,
        band.1,
    )
}

impl VocalStyleProfile for ConversationalProfile {
    type LineExtras = ();

    fn init_prev_pitch(&self, ctx: &VocalContext, band: (u8, u8)) -> u8 {
        conversational_speaking_pitch(ctx, band)
    }

    fn begin_line(&mut self, _rng: &mut XorShift, _ctx: &VocalContext, _line: &LineState<()>) {}

    fn rubato_max(&self, line: &LineState<()>, _s: u32, _slot: f32) -> f32 {
        line.beat_step * 0.10
    }

    fn pick_pitch(
        &self,
        ctx: &VocalContext,
        line: &LineState<()>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
    ) -> u8 {
        // Pitch repetition with bursts. The two repeat branches look
        // collapsible but they're *not*: each performs an independent
        // rng draw and the second only runs when the first failed.
        // Merging them would advance rng one fewer step per syllable
        // and the deterministic output for Conversational would
        // drift. The first branch (~10 %) flags a "spoken-emphasis
        // run" where the next syllable also re-uses prev_pitch; the
        // second branch (~55 % of the remaining 90 %) is the regular
        // repeat. Order of draws matches `derive_conversational`.
        #[allow(clippy::if_same_then_else)]
        let pitch_pre = if rng.next_f32() < 0.10 {
            inp.prev_pitch
        } else if rng.next_f32() < 0.55 {
            inp.prev_pitch
        } else {
            let dir: i16 = if rng.next_f32() < 0.5 { 1 } else { -1 };
            let step: i16 = if rng.next_f32() < 0.18 { 2 } else { 1 };
            ((inp.prev_pitch as i16 + dir * step).clamp(line.band_lo as i16, line.band_hi as i16))
                as u8
        };

        let speaking = conversational_speaking_pitch(ctx, (line.band_lo, line.band_hi));
        let inflected = if inp.s == 0 {
            ((speaking as i16 + 1).clamp(line.band_lo as i16, line.band_hi as i16)) as u8
        } else if inp.s + 1 == line.line_syl {
            ((speaking as i16 - 1).clamp(line.band_lo as i16, line.band_hi as i16)) as u8
        } else {
            pitch_pre
        };

        let strong = inp.s == 0
            || inp.s + 1 == line.line_syl
            || beat_strength(inp.beat_round, ctx.beats_per_bar) >= 0.65;
        if inp.is_final {
            cadence_pitch(
                phrase_role(line.line_idx),
                inp.chord,
                ctx.scale,
                inp.prev_pitch,
                (line.band_lo, line.band_hi),
            )
            .unwrap_or_else(|| {
                inp.chord
                    .and_then(|c| {
                        chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), inflected)
                    })
                    .unwrap_or(inflected)
            })
        } else if strong {
            inp.chord
                .and_then(|c| chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), inflected))
                .unwrap_or(inflected)
        } else if ctx.params.stay_in_scale {
            snap_to_scale(inflected, ctx.scale, line.band_lo, line.band_hi)
        } else {
            inflected
        }
    }

    fn dur_beats(
        &self,
        line: &LineState<()>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        let trim = 0.95 - 0.45 * line.articulation;
        if inp.is_final {
            terminal_dur_beats(line.beat_step, line.articulation)
        } else {
            let trim_local = rhythm_trim(rng, trim, inp.beat_round, beats_per_bar, 0.22);
            line.beat_step * trim_local
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
            &VelocityShape {
                base: 0.62,
                arch: 0.6,
                accent: 0.5,
                jitter: 0.06,
            },
            inp.progress_in_line,
            inp.beat_round,
            beats_per_bar,
        )
    }
}
