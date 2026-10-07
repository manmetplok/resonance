//! Syllable counting for lyric text.
//!
//! One responsibility: turning a lyric line's text into a syllable
//! count. Used by the SVS pipeline, by `VocalContext` and by the phrase
//! spans, all of which need to line notes up with syllables.

use crate::derive::LyricLine;
use crate::g2p::{note_units, resolve_draft, SyllableStress};

/// Lexical stress for each note one lyric line fills, padded with
/// unstressed (or truncated) to `count` notes so indices line up with
/// [`count_syllables`]. A `_` hold is a note too and sings unstressed —
/// it is a held vowel, not a fresh attack.
pub(in crate::derive::vocal) fn line_note_stresses(
    line: &LyricLine,
    count: u32,
) -> Vec<SyllableStress> {
    let mut stresses: Vec<SyllableStress> = resolve_draft(std::slice::from_ref(line))
        .into_iter()
        .flat_map(|s| {
            std::iter::once(s.stress)
                .chain(std::iter::repeat_n(SyllableStress::None, s.slurs_after))
        })
        .collect();
    stresses.resize(count as usize, SyllableStress::None);
    stresses
}

/// The number of notes a lyric text fills, counted by the G2P resolver's
/// own tokenizer so the count and the resolved draft
/// ([`note_unit_count`](crate::g2p::note_unit_count)) always agree:
///
/// * a word fills one note per `·` piece; a between-letter `-` is the
///   same break (`re-so-lu-tion` is four);
/// * an inline `[..]` phoneme block fills one note per `·` group
///   (`[l ih · l iy]` is two), however many phonemes it holds;
/// * a standalone `_` (or `+`) is a melisma hold and fills one note,
///   except before the line's first sung token, where there is nothing
///   to hold and the resolver drops it;
/// * a standalone uppercase `AP` is a breath and fills none, and so does
///   a punctuation-only token (`&`, a standalone `-`).
///
/// No dictionary lookup is involved: unbroken words count as one note
/// whatever CMU says (ingest points auto-syllabify them first). Returns
/// at least 1, so an empty or punctuation-only line still gets a note.
pub fn count_syllables(text: &str) -> u32 {
    (note_units(text).min(u32::MAX as usize) as u32).max(1)
}
