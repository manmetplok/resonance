//! Folk: pentatonic, descending-leaning phrases with long-short
//! rhythm pairs (odd syllables stretched, even compressed). Lines 2
//! and 3 echo the pitch contour of lines 0 and 1 — the call-and-
//! response shape characteristic of folk songs. Lines 0 and 1 store
//! their pitch-offset memory in `echo_offsets`; lines 2+ read it
//! back through the line index mod 2.

use crate::rng::XorShift;

use super::super::params::VocalParams;
use super::super::VocalContext;
use super::{
    beat_strength, cadence_pitch, chord_tone_nearest, phrase_role, rhythm_trim, shape_velocity,
    snap_to_pentatonic, terminal_dur_beats, LineState, StepInputs, VelocityShape,
    VocalStyleProfile,
};

#[derive(Default)]
pub(super) struct FolkProfile {
    /// Per-line pitch offsets (signed semitones relative to
    /// `line_first_pitch`) for line 0 and line 1. Populated by
    /// `end_line` so the second pair of lines can echo them.
    echo_offsets: [Vec<i16>; 2],
}

#[derive(Default, Clone)]
pub(super) struct FolkLine {
    pair_unit: f32,
    long_ratio: f32,
    short_ratio: f32,
    start_pitch: u8,
    descend_span: f32,
    /// Captured at `begin_line` so `pick_pitch` can read it without
    /// borrowing `&mut self`.
    echo_source: Option<Vec<i16>>,
}

impl VocalStyleProfile for FolkProfile {
    type LineExtras = FolkLine;

    fn init_prev_pitch(&self, ctx: &VocalContext, band: (u8, u8)) -> u8 {
        let span = band.1 as i16 - band.0 as i16;
        let start_pitch =
            (band.0 as i16 + (span * 3) / 4).clamp(band.0 as i16, band.1 as i16) as u8;
        snap_to_pentatonic(start_pitch, ctx.scale, band.0, band.1)
    }

    fn breath_frac(&self, params: &VocalParams) -> f32 {
        params.breath.clamp(0.0, 0.9).max(0.20)
    }

    fn begin_line(
        &mut self,
        rng: &mut XorShift,
        _ctx: &VocalContext,
        line: &LineState<FolkLine>,
    ) -> FolkLine {
        // Per-line long-short ratio jitter — one rng draw, must
        // happen before any syllable draws to match the original
        // `derive_folk` sequence.
        let long_ratio = 1.35 + (rng.next_f32() - 0.5) * 0.40;
        let short_ratio = 2.0 - long_ratio;
        let pair_unit = line.sing_span / line.line_syl as f32;

        // Echo source is line_idx >= 2's line (line_idx % 2). The
        // legacy code used `.filter(|v| !v.is_empty())` to gate the
        // echo when the previous line was empty.
        let echo_source: Option<Vec<i16>> = if line.line_idx >= 2 {
            self.echo_offsets
                .get(line.line_idx % 2)
                .filter(|v| !v.is_empty())
                .cloned()
        } else {
            None
        };

        let span = line.band_hi as i16 - line.band_lo as i16;
        let start_pitch = (line.band_lo as i16 + (span * 3) / 4)
            .clamp(line.band_lo as i16, line.band_hi as i16) as u8;
        FolkLine {
            pair_unit,
            long_ratio,
            short_ratio,
            start_pitch,
            descend_span: span as f32 * 0.45,
            echo_source,
        }
    }

    fn slot(&self, line: &LineState<FolkLine>, s: u32) -> f32 {
        if s.is_multiple_of(2) {
            line.extras.pair_unit * line.extras.long_ratio
        } else {
            line.extras.pair_unit * line.extras.short_ratio
        }
    }

    fn pick_pitch(
        &self,
        ctx: &VocalContext,
        line: &LineState<FolkLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
    ) -> u8 {
        let candidate = if let Some(source) = line.extras.echo_source.as_ref() {
            let mut off = source.get(inp.s as usize).copied().unwrap_or(0);
            if rng.next_f32() < 0.25 {
                off += if rng.next_f32() < 0.5 { 1 } else { -1 };
            }
            ((line.line_first_pitch as i16 + off)
                .clamp(line.band_lo as i16, line.band_hi as i16)) as u8
        } else {
            let descend_target = (line.extras.start_pitch as f32
                - inp.progress_in_line * line.extras.descend_span)
                .clamp(line.band_lo as f32, line.band_hi as f32)
                as u8;
            // Order of draws matches `derive_folk`: 0.05 first, then
            // 0.35; the inner `0.5` only fires if 0.35 succeeded.
            let jitter = if rng.next_f32() < 0.05 {
                -((rng.next_range(3) as i16) + 3) // -3..-5
            } else if rng.next_f32() < 0.35 {
                if rng.next_f32() < 0.5 { 1 } else { -1 }
            } else {
                0
            };
            ((descend_target as i16 + jitter).clamp(line.band_lo as i16, line.band_hi as i16))
                as u8
        };

        let is_long = inp.s.is_multiple_of(2);
        let strong = inp.s == 0
            || inp.s + 1 == line.line_syl
            || (is_long && beat_strength(inp.beat_round, ctx.beats_per_bar) >= 0.65);
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
                        chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), candidate)
                    })
                    .unwrap_or(candidate)
            })
        } else if strong {
            inp.chord
                .and_then(|c| chord_tone_nearest(c.chord, (line.band_lo, line.band_hi), candidate))
                .unwrap_or(candidate)
        } else {
            snap_to_pentatonic(candidate, ctx.scale, line.band_lo, line.band_hi)
        }
    }

    fn dur_beats(
        &self,
        line: &LineState<FolkLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        let trim = 0.92 - 0.40 * line.articulation;
        if inp.is_final {
            terminal_dur_beats(line.extras.pair_unit, line.articulation)
        } else {
            let trim_local = rhythm_trim(rng, trim, inp.beat_round, beats_per_bar, 0.12);
            inp.slot * trim_local
        }
    }

    fn velocity(
        &self,
        _line: &LineState<FolkLine>,
        inp: &StepInputs<'_>,
        rng: &mut XorShift,
        beats_per_bar: u32,
    ) -> f32 {
        shape_velocity(
            rng,
            &VelocityShape {
                base: 0.70,
                arch: 0.7,
                accent: 0.85,
                jitter: 0.10,
            },
            inp.progress_in_line,
            inp.beat_round,
            beats_per_bar,
        )
    }

    fn end_line(&mut self, line_idx: usize, line_offsets: Vec<i16>) {
        if line_idx < 2 {
            self.echo_offsets[line_idx] = line_offsets;
        }
    }
}
