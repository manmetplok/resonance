//! Syllable counting for lyric text.
//!
//! One responsibility: turning a lyric line's text into a syllable
//! count. Used by the SVS pipeline, by `VocalContext` and by the phrase
//! spans, all of which need to line notes up with syllables.

/// Strip the syllable separator and count syllables in a lyric line. A
/// fallback for cases where `LyricLine::syllables` is 0.
pub fn count_syllables(text: &str) -> u32 {
    let dot_count = text.matches('\u{00B7}').count() as u32;
    // `n syllables = dot_count + word_count` is a reasonable approximation
    // for already-broken text; we add the dots to the word count.
    let word_count = text.split_whitespace().count() as u32;
    (dot_count + word_count).max(1)
}
