//! Melisma holds: where a slur note's hold comes from, and editing the
//! draft's `_` holds in place.
//!
//! A slur note is held for one of two reasons (vocal-mcp-control-and-
//! quality.md §6):
//!
//! * the **draft** writes a `_` hold (the user's — typed, or put there by
//!   the vocal roll's `s` key), or
//! * the clip's per-note side table marks the note `+` — what the melody
//!   generator does for the holds it makes itself (`SyllableMode` Mixed /
//!   Melismatic), regenerated with every melody and never written into
//!   the lyrics.
//!
//! [`hold_origins`] tells the two apart with the same walk the note
//! assignment uses; [`set_holds_after`] rewrites the draft text so a
//! syllable carries an exact number of `_` holds.

use super::lyric_parse::{is_slur_token, tokenize_line, LyricToken};
use super::note_assign::{assign_syllables_to_notes, resolve_draft, ResolvedSyllable};
use super::syllabify::normalize_hyphen_marks;
use crate::derive::LyricLine;

/// Why a slur note holds its syllable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldOrigin {
    /// A `_` hold written in the lyric draft.
    Draft,
    /// A per-note slur annotation in the clip's side table — the melody
    /// generator's own holds.
    Annotation,
}

/// Per note, `Some(origin)` for a slur note and `None` for a sung one.
///
/// Mirrors [`assign_syllables_to_notes`]: after a syllable is sung its
/// draft holds (`slurs_after`) are owed to the next notes, and a slur
/// note that uses one up is a [`HoldOrigin::Draft`] hold even when it is
/// also annotated (the annotation is the same hold). A slur note with no
/// draft hold owed is held only by its annotation.
pub fn hold_origins(
    syllables: &[ResolvedSyllable],
    annotations: &[String],
    note_count: usize,
) -> Vec<Option<HoldOrigin>> {
    let assigned = assign_syllables_to_notes(syllables, annotations, note_count);
    let mut owed = 0usize;
    assigned
        .iter()
        .map(|note| {
            if !note.is_slur {
                owed = syllables.get(note.syllable_index).map_or(0, |s| s.slurs_after);
                None
            } else if owed > 0 {
                owed -= 1;
                Some(HoldOrigin::Draft)
            } else {
                Some(HoldOrigin::Annotation)
            }
        })
        .collect()
}

/// The resolved-syllable index of each draft line's first syllable,
/// for the lines that have one (a punctuation-only line has none).
pub fn line_first_syllables(draft: &[LyricLine]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut next = 0usize;
    for line in draft {
        let count = resolve_draft(std::slice::from_ref(line)).len();
        if count > 0 {
            out.push(next);
        }
        next += count;
    }
    out
}

/// Rewrite the draft so the resolved syllable `syllable_index` carries
/// exactly `holds` `_` holds. A word's last syllable (or a `[..]` block's
/// last group) takes standalone ` _` tokens after it (`love _ _`); a
/// syllable inside a word takes in-word hold pieces (`a·_·ma·zing`). The
/// line's `syllables` count follows the text.
///
/// Returns `false` and leaves the draft untouched when the syllable does
/// not exist, sits inside a `[..]` block, or the text is shaped so that
/// the rewrite would not resolve to the asked-for count.
pub fn set_holds_after(draft: &mut [LyricLine], syllable_index: usize, holds: usize) -> bool {
    let mut first = 0usize;
    for line_idx in 0..draft.len() {
        let line = &draft[line_idx];
        let count = resolve_draft(std::slice::from_ref(line)).len();
        if syllable_index >= first + count {
            first += count;
            continue;
        }
        let Some(text) = rewrite_line(&line.text, syllable_index - first, holds) else {
            return false;
        };
        let old = std::mem::replace(&mut draft[line_idx].text, text);
        let resolved = resolve_draft(draft);
        if resolved.get(syllable_index).map(|s| s.slurs_after) != Some(holds) {
            draft[line_idx].text = old;
            return false;
        }
        let line = &mut draft[line_idx];
        line.syllables = crate::count_syllables(&line.text).min(255) as u8;
        return true;
    }
    false
}

/// A whitespace-delimited token of a lyric line (a `[..]` block counts
/// as one even with spaces inside), as a byte range.
fn token_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some(&(start, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let mut end = start;
        if c == '[' {
            for (i, ch) in chars.by_ref() {
                end = i + ch.len_utf8();
                if ch == ']' {
                    break;
                }
            }
        } else {
            while let Some(&(i, ch)) = chars.peek() {
                if ch.is_whitespace() || (ch == '[' && i != start) {
                    break;
                }
                end = i + ch.len_utf8();
                chars.next();
            }
        }
        spans.push((start, end));
    }
    spans
}

/// Sung syllables in one token, and whether it is a `[..]` block.
fn token_syllables(token: &str) -> (usize, bool) {
    tokenize_line(token).iter().fold((0, false), |(n, block), t| match t {
        LyricToken::Word { pieces, .. } => (n + pieces.len(), block),
        LyricToken::PhonemeBlock(groups) => (n + groups.len(), true),
        LyricToken::Slur | LyricToken::Breath => (n, block),
    })
}

/// `rewrite_line` for the `local`-th syllable of one line.
fn rewrite_line(text: &str, local: usize, holds: usize) -> Option<String> {
    let spans = token_spans(text);
    let mut seen = 0usize;
    for (pos, &(start, end)) in spans.iter().enumerate() {
        let token = &text[start..end];
        let (count, block) = token_syllables(token);
        if local >= seen + count {
            seen += count;
            continue;
        }
        let piece = local - seen;
        if piece + 1 < count {
            // Inside a word: in-word hold pieces. A block cannot hold
            // mid-way.
            if block {
                return None;
            }
            let word = with_in_word_holds(token, piece, holds)?;
            return Some(format!("{}{}{}", &text[..start], word, &text[end..]));
        }
        // The token's last syllable: drop its trailing in-word holds and
        // the standalone holds after it, then write `holds` fresh ones.
        let word = if block { token.to_owned() } else { without_trailing_holds(token) };
        let mut tail_end = end;
        for &(s, e) in &spans[pos + 1..] {
            if !is_slur_token(&text[s..e]) {
                break;
            }
            tail_end = e;
        }
        let mut out = String::with_capacity(text.len() + 2 * holds);
        out.push_str(&text[..start]);
        out.push_str(&word);
        for _ in 0..holds {
            out.push_str(" _");
        }
        out.push_str(&text[tail_end..]);
        return Some(out);
    }
    None
}

/// `word` with the hold pieces after its last real piece removed.
fn without_trailing_holds(word: &str) -> String {
    let normalized = normalize_hyphen_marks(word);
    let mut segments: Vec<&str> = normalized.split('\u{00B7}').collect();
    while segments.len() > 1 && segments.last().is_some_and(|s| is_slur_token(s)) {
        segments.pop();
    }
    segments.join("\u{00B7}")
}

/// `text` without its `·` syllable breaks — the plain prose the bulk
/// lyric editor shows. A break next to an in-word hold piece stays
/// (`a·_·ma·zing` → `a·_·mazing`), so the hold survives the editor
/// writing the text back.
pub fn strip_syllable_marks(text: &str) -> String {
    let segments: Vec<&str> = text.split('\u{00B7}').collect();
    let mut out = String::with_capacity(text.len());
    for (i, segment) in segments.iter().enumerate() {
        if i > 0 {
            let hold = |s: &str| is_slur_token(s.trim());
            if hold(segments[i - 1]) || hold(segment) {
                out.push('\u{00B7}');
            }
        }
        out.push_str(segment);
    }
    out.replace("  ", " ")
}

/// `word` with exactly `holds` hold pieces after its `piece`-th real
/// piece.
fn with_in_word_holds(word: &str, piece: usize, holds: usize) -> Option<String> {
    let normalized = normalize_hyphen_marks(word);
    let mut out: Vec<&str> = Vec::new();
    let mut real = 0usize;
    let mut skipping = false;
    let mut placed = false;
    for segment in normalized.split('\u{00B7}') {
        if is_slur_token(segment) {
            if skipping {
                continue;
            }
            out.push(segment);
            continue;
        }
        skipping = false;
        out.push(segment);
        if real == piece {
            out.extend(std::iter::repeat_n("_", holds));
            skipping = true;
            placed = true;
        }
        real += 1;
    }
    placed.then(|| out.join("\u{00B7}"))
}
