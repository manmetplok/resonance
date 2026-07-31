//! Lyric tokenizer and inline phoneme-block parser.
//!
//! Recognises `[hh ah l ow]`-style overrides embedded in lyric text
//! and handles the `(N)` pronunciation-variant suffix.

use super::phonemes::canonical_phoneme;

/// Each lyric token resolved out of the draft. Either a normal English
/// word (CMU-lookup + split) or an explicit phoneme block the user
/// typed between square brackets (`[hh ah l ow]`) to override
/// pronunciation for proper nouns, foreign words, or anything CMU
/// gets wrong. Internal to the note-assignment resolver.
pub(crate) enum LyricToken {
    Word { cleaned: String, syl_count: usize, variant_idx: usize },
    /// Pre-segmented phoneme groups, one inner vec per syllable.
    /// User-typed overrides carry no stress, so the syllable stress
    /// defaults to `None`.
    PhonemeBlock(Vec<Vec<&'static str>>),
}

/// Walk a line and split it into `LyricToken`s. Recognises `[...]`
/// blocks as inline phoneme overrides; everything else is a word.
pub(crate) fn tokenize_line(text: &str) -> Vec<LyricToken> {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_control()).collect();
    let mut tokens: Vec<LyricToken> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '[' {
            // Scan for closing `]`.
            let mut j = i + 1;
            while j < chars.len() && chars[j] != ']' {
                j += 1;
            }
            if j < chars.len() && chars[j] == ']' {
                let inner: String = chars[(i + 1)..j].iter().collect();
                let groups = parse_phoneme_block(&inner);
                if !groups.is_empty() {
                    tokens.push(LyricToken::PhonemeBlock(groups));
                }
                i = j + 1;
                continue;
            }
            // Unclosed bracket — skip the stray `[` and the rest of
            // the line's text gets tokenized normally. Without this
            // advance the plain-word scan below (which stops at `[`)
            // would never move past the bracket and we'd infinite-
            // loop.
            i += 1;
            continue;
        }
        // Plain word: scan to the next whitespace or `[`.
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '[' {
            i += 1;
        }
        let word_raw: String = chars[start..i].iter().collect();
        // Extract a trailing `(N)` pronunciation hint. Recognised
        // pattern: end of token, `(`, one or more digits, `)`. The
        // hint lives outside the phonemic word so we strip it before
        // syllable + cleanup processing.
        let (word_body, variant_idx) = extract_variant_hint(&word_raw);
        // Hand-typed `re-so-lu-tion` is the same instruction as
        // `re·so·lu·tion`; normalise before counting so a lyric written
        // with plain ASCII hyphens splits across notes too.
        let word_body = super::syllabify::normalize_hyphen_marks(&word_body);
        let trimmed = word_body.trim_matches(|c: char| {
            !c.is_alphabetic() && c != '\'' && c != '\u{00B7}'
        });
        let syl_count = trimmed.split('\u{00B7}').count().max(1);
        let cleaned: String = trimmed
            .chars()
            .filter(|c| c.is_alphabetic() || *c == '\'')
            .collect();
        if !cleaned.is_empty() {
            tokens.push(LyricToken::Word { cleaned, syl_count, variant_idx });
        }
    }
    tokens
}

/// Strip a trailing `(N)` from `word` and return `(stripped, variant)`.
/// `variant` defaults to 1 when no hint is present. Anything other
/// than a digit run inside the parens is preserved unchanged.
fn extract_variant_hint(word: &str) -> (String, usize) {
    if !word.ends_with(')') {
        return (word.to_string(), 1);
    }
    if let Some(open) = word.rfind('(') {
        let inside = &word[open + 1..word.len() - 1];
        if !inside.is_empty() && inside.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(n) = inside.parse::<usize>() {
                return (word[..open].to_string(), n.max(1));
            }
        }
    }
    (word.to_string(), 1)
}

/// Parse the contents of a `[...]` block into per-syllable phoneme
/// groups. Phonemes are whitespace-separated; `·` between phonemes
/// marks a syllable boundary, so `l ih · l iy · ah` is three
/// syllables. Unknown phonemes are silently dropped — the resulting
/// chunk just gets fewer phonemes, no crash.
fn parse_phoneme_block(inner: &str) -> Vec<Vec<&'static str>> {
    let mut groups: Vec<Vec<&'static str>> = vec![Vec::new()];
    for tok in inner.split(|c: char| c.is_whitespace()) {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        if tok == "\u{00B7}" {
            groups.push(Vec::new());
            continue;
        }
        // Inline mid-token `·` (e.g. `ih·l` with no spaces).
        if tok.contains('\u{00B7}') {
            let parts: Vec<&str> = tok.split('\u{00B7}').collect();
            for (k, part) in parts.iter().enumerate() {
                if k > 0 {
                    groups.push(Vec::new());
                }
                if let Some(canon) = canonical_phoneme(part) {
                    groups.last_mut().unwrap().push(canon);
                }
            }
            continue;
        }
        if let Some(canon) = canonical_phoneme(tok) {
            groups.last_mut().unwrap().push(canon);
        }
    }
    // Drop empty groups (e.g. trailing `·` with nothing after).
    groups.retain(|g| !g.is_empty());
    groups
}

/// OpenUtau-style slur marker. A note whose lyric equals this (or `-`)
/// continues the previous syllable's vowel rather than starting a new
/// attack. Centralised so the GUI, the SVS pipeline, and the lyric
/// side-table never disagree on which sigil counts.
pub const SLUR_MARKER: &str = "+";

/// `true` when `s` is a slur annotation (`"+"` or `"-"`, ignoring
/// surrounding whitespace). The single source of truth for the
/// convention — every call site routes through this.
pub fn is_slur_lyric(s: &str) -> bool {
    let t = s.trim();
    t == "+" || t == "-"
}
