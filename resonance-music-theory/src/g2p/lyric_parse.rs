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
    /// A sung word. `pieces` holds the word split at the user's `·`
    /// (or between-letter `-`) breaks, each piece reduced to letters and
    /// apostrophes; an unbroken word is one piece. `cleaned` is the
    /// pieces joined — the dictionary key. `variant_idx` is the `(N)`
    /// pronunciation hint, `None` when the word carries none.
    Word { cleaned: String, pieces: Vec<String>, variant_idx: Option<usize> },
    /// Pre-segmented phoneme groups, one inner vec per syllable.
    /// User-typed overrides carry no stress, so the syllable stress
    /// defaults to `None`.
    PhonemeBlock(Vec<Vec<&'static str>>),
    /// A standalone melisma token (`_`, or its alias `+`): hold
    /// the previous syllable onto one more note. Never emitted before the
    /// first sung token of a line — there is nothing to hold yet.
    Slur,
    /// A standalone uppercase `AP`: a breath before the next sung
    /// syllable. Takes no note.
    Breath,
}

/// `true` when a whitespace-delimited lyric token is a standalone
/// melisma marker in the draft: `_`, or its alias `+`. The draft-side
/// counterpart of [`is_slur_lyric`].
///
/// A standalone `-` is deliberately *not* a hold in the draft: prose
/// writes it as a dash (`wait - no`), and reading it as a melisma
/// silently added a note. It is punctuation there and dropped; only the
/// per-note side table ([`is_slur_lyric`]) still accepts `-`, where a
/// lone dash on a note can only mean "hold".
pub fn is_slur_token(token: &str) -> bool {
    matches!(token, "_" | "+")
}

/// `true` when a whitespace-delimited lyric token is the breath marker:
/// exactly `AP`, uppercase. Lowercase `ap` is an ordinary word, and the
/// bracketed `[AP]` is an inline phoneme block, not this marker.
pub fn is_breath_token(token: &str) -> bool {
    token == "AP"
}

/// Walk a line and split it into `LyricToken`s. Recognises `[...]`
/// blocks as inline phoneme overrides, standalone `_` / `+` as
/// slurs and standalone `AP` as a breath; everything else is a word.
/// A slur before the line's first sung token is dropped.
pub(crate) fn tokenize_line(text: &str) -> Vec<LyricToken> {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_control()).collect();
    let mut tokens: Vec<LyricToken> = Vec::new();
    let mut sung_seen = false;
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
                    sung_seen = true;
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
        if is_slur_token(&word_raw) {
            if sung_seen {
                tokens.push(LyricToken::Slur);
            }
            continue;
        }
        if is_breath_token(&word_raw) {
            tokens.push(LyricToken::Breath);
            continue;
        }
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
        let pieces: Vec<String> = trimmed
            .split('\u{00B7}')
            .map(|p| p.chars().filter(|c| c.is_alphabetic() || *c == '\'').collect())
            .collect();
        let cleaned: String = pieces.concat();
        if !cleaned.is_empty() {
            tokens.push(LyricToken::Word { cleaned, pieces, variant_idx });
            sung_seen = true;
        }
    }
    tokens
}

/// Strip a trailing `(N)` from `word` and return `(stripped, variant)`.
/// `variant` is `None` when no hint is present. Anything other than a
/// digit run inside the parens is preserved unchanged.
fn extract_variant_hint(word: &str) -> (String, Option<usize>) {
    if !word.ends_with(')') {
        return (word.to_string(), None);
    }
    if let Some(open) = word.rfind('(') {
        let inside = &word[open + 1..word.len() - 1];
        if !inside.is_empty() && inside.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(n) = inside.parse::<usize>() {
                return (word[..open].to_string(), Some(n.max(1)));
            }
        }
    }
    (word.to_string(), None)
}

/// Notes a lyric text fills, counted from the same tokens the resolver
/// walks but without any dictionary lookup: a word fills one note per
/// `·` (or between-letter `-`) piece, an inline `[..]` block one per `·`
/// group, a `_` / `+` hold one, and an `AP` breath none.
/// Punctuation-only tokens (`&`, a standalone `-`) fill nothing, and a
/// hold before a line's first sung token is dropped, exactly as
/// [`resolve_draft`](super::resolve_draft) drops it. `\n` separates
/// lines.
pub(crate) fn note_units(text: &str) -> usize {
    text.split('\n')
        .flat_map(tokenize_line)
        .map(|token| match token {
            LyricToken::Word { pieces, .. } => pieces.len(),
            LyricToken::PhonemeBlock(groups) => groups.len(),
            LyricToken::Slur => 1,
            LyricToken::Breath => 0,
        })
        .sum()
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

/// `true` when `s` is a per-note slur annotation (`"+"`, `"-"` or `"_"`,
/// ignoring surrounding whitespace). The single source of truth for the
/// convention — every call site routes through this. `_` is accepted so
/// the per-note side-table and the lyric draft (see [`is_slur_token`])
/// share one spelling; `-` is accepted here only, never in the draft.
pub fn is_slur_lyric(s: &str) -> bool {
    let s = s.trim();
    is_slur_token(s) || s == "-"
}
