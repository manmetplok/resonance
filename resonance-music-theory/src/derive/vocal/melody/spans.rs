//! Lyric-line phrase spans.
//!
//! One responsibility: recovering where each sung phrase sits in time,
//! for callers (the synth fill) that need to work around the vocal.

use crate::derive::vocal::params::VocalParams;
use crate::derive::GeneratedNote;

use super::syllables::count_syllables;

/// Group `notes` (one per syllable, in lyric order) into per-line
/// `(start_tick, end_tick)` phrase intervals using `params.draft` to
/// recover the lyric line boundaries. Each interval's start is the
/// earliest onset of any note in the line and its end is the latest
/// note's `start_tick + duration_ticks`. Lines with no syllables are
/// skipped.
///
/// Used by `MelodyParams::fill_vocal_gaps`: the synth fill needs to
/// know where the actual sung phrases sit, and the lyric line is the
/// authoritative phrase unit. Time-gap heuristics fail because the
/// vocal generator's `phrase_start_offset` can pull successive lines
/// into each other, leaving only a few-tick gap between them.
pub fn vocal_phrase_spans(notes: &[GeneratedNote], params: &VocalParams) -> Vec<(u64, u64)> {
    let line_syl: Vec<u32> = params
        .draft
        .iter()
        .map(|l| count_syllables(&l.text))
        .collect();
    let mut out = Vec::with_capacity(line_syl.len());
    let mut cursor = 0usize;
    for &n_syl in &line_syl {
        let n = (n_syl as usize).min(notes.len().saturating_sub(cursor));
        if n == 0 {
            continue;
        }
        let slice = &notes[cursor..cursor + n];
        let start = slice.iter().map(|x| x.start_tick).min().unwrap_or(0);
        let end = slice
            .iter()
            .map(|x| x.start_tick + x.duration_ticks)
            .max()
            .unwrap_or(start);
        out.push((start, end));
        cursor += n;
    }
    out
}
