//! Syllable counting for lyric text.
//!
//! One responsibility: turning a lyric line's text into a syllable
//! count. Used by the SVS pipeline, by `VocalContext` and by the phrase
//! spans, all of which need to line notes up with syllables.

use crate::derive::LyricLine;
use crate::g2p::{is_breath_token, is_slur_token, resolve_draft, SyllableStress};

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

/// Strip the syllable separator and count the notes a lyric line fills.
/// A fallback for cases where `LyricLine::syllables` is 0.
///
/// Every whitespace-separated token counts one, plus one per `·` break,
/// with the two draft markers handled the way the G2P resolver handles
/// them, so the count and the resolved draft agree:
///
/// * a standalone `_` (or `+` / `-`) is a melisma hold and counts one
///   note, except before the line's first sung token, where there is
///   nothing to hold and the resolver drops it;
/// * a standalone uppercase `AP` is a breath and counts nothing.
pub fn count_syllables(text: &str) -> u32 {
    let dot_count = text.matches('\u{00B7}').count() as u32;
    // `n syllables = dot_count + word_count` is a reasonable approximation
    // for already-broken text; we add the dots to the word count.
    let mut word_count = 0u32;
    for line in text.split('\n') {
        let mut sung_seen = false;
        for token in line.split_whitespace() {
            if is_breath_token(token) {
                continue;
            }
            if is_slur_token(token) {
                if sung_seen {
                    word_count += 1;
                }
                continue;
            }
            sung_seen = true;
            word_count += 1;
        }
    }
    (dot_count + word_count).max(1)
}
