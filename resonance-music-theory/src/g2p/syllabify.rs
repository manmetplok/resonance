//! Syllabification utilities: counting, inserting `·` markers, and
//! splitting a flat phoneme sequence across syllable boundaries.

use super::phonemes::{is_consonant, word_to_phonemes_variant, SyllableStress};

/// CMU's natural syllable count for a word (== number of vowel
/// phonemes in its CMU pronunciation, or in the rule-based fallback
/// for OOV words). Useful for catching mismatches between the user's
/// `·`-marked syllable count and what the SVS model actually sings:
/// fewer dots than this number causes phonemes to cram into one note.
///
/// Returns at least 1 for any non-empty input.
pub fn cmu_syllable_count(word: &str) -> usize {
    let cleaned: String = word
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphabetic() || *c == '\'')
        .collect();
    if cleaned.is_empty() {
        return 1;
    }
    let phonemes = word_to_phonemes_variant(&cleaned, 1);
    phonemes
        .iter()
        .filter(|(p, _)| !is_consonant(p))
        .count()
        .max(1)
}

/// Insert `·` markers into a single word so it has at least
/// `target_syllables` syllables. Tries to place dots between
/// consonant→vowel transitions in the spelling so each chunk reads
/// naturally (e.g. `library` with target=3 → `li·bra·ry`). Words that
/// already have ≥ target dots are returned unchanged.
///
/// This is a best-effort spelling heuristic — it won't always agree
/// with a dictionary syllabification but it consistently produces a
/// reasonable per-note breakdown for English.
pub fn syllabify_word(word: &str, target_syllables: usize) -> String {
    let existing_dots = word.matches('\u{00B7}').count();
    if existing_dots + 1 >= target_syllables.max(1) {
        return word.to_string();
    }
    let needed = target_syllables - 1 - existing_dots;
    if needed == 0 {
        return word.to_string();
    }
    // Find candidate split points: between a consonant letter and a
    // following vowel letter. English-style onset-maximization places
    // the syllable boundary just BEFORE the consonant cluster that
    // leads into the next vowel.
    let chars: Vec<char> = word.chars().collect();
    let is_vowel = |c: char| matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y');
    let is_letter = |c: char| c.is_alphabetic();
    let mut candidates: Vec<usize> = Vec::new();
    // Walk vowel runs; each run after the first is the start of a new
    // syllable. The split goes BEFORE the consonant cluster preceding
    // that vowel run, so we step back through preceding consonants.
    let mut in_vowel_run = false;
    let mut seen_first_vowel_run = false;
    for (i, &c) in chars.iter().enumerate() {
        if !is_letter(c) {
            in_vowel_run = false;
            continue;
        }
        if is_vowel(c) {
            if !in_vowel_run {
                if seen_first_vowel_run {
                    // New vowel run — the boundary goes before the
                    // consonants that start it. Step backward over the
                    // preceding consonant cluster, then hand the next
                    // syllable only as much of it as English would let a
                    // syllable start with: the whole cluster if it is one
                    // or two letters forming a real onset, otherwise just
                    // the last letter. Without the limit `function` split
                    // as `fu·nction` — cosmetically wrong on the note, and
                    // out of step with the phoneme split, which does the
                    // same phonotactic check.
                    let mut k = i;
                    while k > 0 && is_letter(chars[k - 1]) && !is_vowel(chars[k - 1]) {
                        k -= 1;
                    }
                    let cluster_len = i - k;
                    if cluster_len > 2
                        || (cluster_len == 2 && !is_onset_digraph(chars[k], chars[k + 1]))
                    {
                        k = i - 1;
                    }
                    if k > 0 && k < chars.len() {
                        candidates.push(k);
                    }
                }
                seen_first_vowel_run = true;
            }
            in_vowel_run = true;
        } else {
            in_vowel_run = false;
        }
    }
    if candidates.is_empty() {
        return word.to_string();
    }
    // Pick `needed` candidates — spread evenly to cover the word.
    let take = needed.min(candidates.len());
    let stride = (candidates.len() as f32 / take as f32).max(1.0);
    let mut chosen: Vec<usize> = (0..take)
        .map(|k| {
            let pos = (k as f32 * stride).round() as usize;
            candidates[pos.min(candidates.len() - 1)]
        })
        .collect();
    chosen.sort_unstable();
    chosen.dedup();
    // Insert dots from the back so earlier indices stay valid.
    let mut out: Vec<char> = chars.clone();
    for &pos in chosen.iter().rev() {
        out.insert(pos, '\u{00B7}');
    }
    out.into_iter().collect()
}

/// Can this two-letter spelling start an English syllable? The spelling
/// counterpart of [`is_legal_onset_pair`], used only to place the display
/// `·` sensibly (`func·tion`, not `fu·nction`).
fn is_onset_digraph(a: char, b: char) -> bool {
    let pair = [a.to_ascii_lowercase(), b.to_ascii_lowercase()];
    matches!(
        pair.iter().collect::<String>().as_str(),
        "bl" | "br"
            | "ch"
            | "cl"
            | "cr"
            | "dr"
            | "dw"
            | "fl"
            | "fr"
            | "gl"
            | "gr"
            | "kn"
            | "ph"
            | "pl"
            | "pr"
            | "qu"
            | "sc"
            | "sh"
            | "sk"
            | "sl"
            | "sm"
            | "sn"
            | "sp"
            | "st"
            | "sw"
            | "th"
            | "tr"
            | "tw"
            | "wh"
            | "wr"
    )
}

/// The ASCII hyphen doubles as a hand-written syllable break, so a
/// caller can type `re-so-lu-tion` instead of hunting for `·` on their
/// keyboard. Recognised everywhere `·` is (see [`auto_syllabify_text`]
/// and the lyric tokenizer), and normalised to `·` on ingest so only one
/// marker ever reaches the resolver.
pub const HYPHEN_SYLLABLE_MARKER: char = '-';

/// `true` when `word` already carries an explicit syllable break — a `·`
/// the auto-syllabifier (or the user) inserted, or a hand-typed `-`
/// *between two letters* (a leading/trailing dash is punctuation, not a
/// break). Explicit breaks are authoritative: [`auto_syllabify_text`]
/// leaves such a word exactly as written.
pub fn has_syllable_marks(word: &str) -> bool {
    if word.contains('\u{00B7}') {
        return true;
    }
    let chars: Vec<char> = word.chars().collect();
    chars.iter().enumerate().any(|(i, &c)| {
        c == HYPHEN_SYLLABLE_MARKER
            && i > 0
            && i + 1 < chars.len()
            && chars[i - 1].is_alphabetic()
            && chars[i + 1].is_alphabetic()
    })
}

/// Rewrite hand-typed `-` syllable breaks to the canonical `·`. Only
/// hyphens between two letters are converted, so `"well-known"` becomes
/// the two-syllable `"well·known"` (which is what you want to sing) while
/// a dangling `"—"`-style dash stays punctuation the word cleaner drops.
pub(super) fn normalize_hyphen_marks(word: &str) -> String {
    let chars: Vec<char> = word.chars().collect();
    chars
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let is_break = c == HYPHEN_SYLLABLE_MARKER
                && i > 0
                && i + 1 < chars.len()
                && chars[i - 1].is_alphabetic()
                && chars[i + 1].is_alphabetic();
            if is_break {
                '\u{00B7}'
            } else {
                c
            }
        })
        .collect()
}

/// Insert `·` markers into a whole lyric text so each word matches CMU's
/// syllable count — the single normalisation every lyric ingest point
/// runs, so a multi-syllable word lands on one note *per syllable*
/// instead of cramming its whole phoneme run onto one note.
///
/// Three things are deliberately left alone:
///
/// * **Line structure.** `\n` is preserved, so bulk lyric text (one line
///   per lyric line) round-trips. Only intra-line whitespace runs are
///   collapsed to a single space.
/// * **`[...]` phoneme blocks.** A power-user override like
///   `[l ih · l iy]` is copied through verbatim — its inner tokens are
///   phonemes, not spellings, and syllabifying them would corrupt the
///   override.
/// * **Words that already carry a break** (`·` or a hand-typed
///   `re-so-lu-tion`), preserving user-intentional melismas and manual
///   hyphenation. Hand-typed `-` breaks are normalised to `·`.
pub fn auto_syllabify_text(text: &str) -> String {
    walk_lyric_text(text, true)
}

/// The notation-only half of [`auto_syllabify_text`]: rewrite hand-typed
/// `-` syllable breaks to `·` and change nothing else — no automatic
/// splitting. For a caller that wants to place every break itself but
/// still type them with an ASCII keyboard.
pub fn normalize_syllable_marks(text: &str) -> String {
    walk_lyric_text(text, false)
}

/// Shared walk for the two entry points above: per line, per word,
/// skipping `[...]` phoneme blocks. `syllabify` decides whether a word
/// with no explicit break gets one inserted.
fn walk_lyric_text(text: &str, syllabify: bool) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for (line_idx, line) in text.split('\n').enumerate() {
        if line_idx > 0 {
            out.push('\n');
        }
        out.push_str(&walk_lyric_line(line, syllabify));
    }
    out
}

/// One line of [`walk_lyric_text`]. Split out so the bracket scan never
/// has to reason about newlines.
fn walk_lyric_line(text: &str, syllabify: bool) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut first = true;
    // `[...]` blocks are copied verbatim. They contain spaces, so we
    // cannot simply walk `split_whitespace`: track bracket depth and
    // pass everything between `[` and `]` straight through.
    let mut in_block = false;
    for word_raw in text.split_whitespace() {
        if !first {
            out.push(' ');
        }
        first = false;
        if in_block {
            out.push_str(word_raw);
            if word_raw.contains(']') {
                in_block = false;
            }
            continue;
        }
        if word_raw.contains('[') {
            out.push_str(word_raw);
            // A single-token block (`[hh]`) opens and closes at once.
            in_block = !word_raw.contains(']');
            continue;
        }
        // An explicit break wins: normalise `-` to `·` and stop. Same
        // path when automatic splitting is off — the caller is placing
        // every break itself.
        if !syllabify || has_syllable_marks(word_raw) {
            out.push_str(&normalize_hyphen_marks(word_raw));
            continue;
        }
        // Strip leading/trailing non-letter punctuation so we can ask
        // CMU about the bare word, then put the punctuation back.
        let lead_count = word_raw
            .chars()
            .take_while(|c| !c.is_alphabetic() && *c != '\'' && *c != '\u{00B7}')
            .count();
        let trail_count = word_raw
            .chars()
            .rev()
            .take_while(|c| !c.is_alphabetic() && *c != '\'' && *c != '\u{00B7}')
            .count();
        let lead: String = word_raw.chars().take(lead_count).collect();
        let body_len = word_raw.chars().count() - lead_count - trail_count;
        let body: String = word_raw.chars().skip(lead_count).take(body_len).collect();
        let trail: String = word_raw.chars().skip(lead_count + body_len).collect();
        let target = cmu_syllable_count(&body);
        let syllabified = syllabify_word(&body, target);
        out.push_str(&lead);
        out.push_str(&syllabified);
        out.push_str(&trail);
    }
    out
}

/// Split a phoneme list into `n` syllable-shaped chunks, one vowel
/// nucleus each. Consonants between two vowels are divided by
/// [`syllable_boundary`]: as many as English allows become the next
/// syllable's onset (`hou·ses` → `hh aw / s ax z`), the rest stay as the
/// previous syllable's coda (`func·tion` → `f ah ng k / sh ax n`).
/// Operates on `(phoneme, stress)` pairs so the stress on each vowel
/// travels with the chunk it ends up in.
pub(crate) fn split_into_syllables(
    phonemes: &[(&'static str, SyllableStress)],
    n: usize,
) -> Vec<Vec<(&'static str, SyllableStress)>> {
    if n <= 1 {
        return vec![phonemes.to_vec()];
    }
    // Find vowel positions.
    let vowels: Vec<usize> = phonemes
        .iter()
        .enumerate()
        .filter(|(_, (p, _))| !is_consonant(p))
        .map(|(i, _)| i)
        .collect();
    if vowels.len() < n {
        // Not enough vowels — emit one chunk per requested syllable
        // by spreading the phonemes evenly. Filler vowels are inserted
        // unstressed.
        let mut out = Vec::with_capacity(n);
        let chunk_size = phonemes.len().max(1) / n.max(1);
        for k in 0..n {
            let start = k * chunk_size;
            let end = if k == n - 1 {
                phonemes.len()
            } else {
                (k + 1) * chunk_size
            };
            let chunk: Vec<(&'static str, SyllableStress)> =
                phonemes[start..end.min(phonemes.len())].to_vec();
            if chunk.is_empty() {
                out.push(vec![("ah", SyllableStress::None)]);
            } else {
                out.push(chunk);
            }
        }
        return out;
    }

    // We have at least n vowels. Take the first n vowels as syllable
    // nuclei and cut each intervocalic consonant run at the point
    // `syllable_boundary` picks.
    let chosen_vowels: Vec<usize> = vowels.iter().copied().take(n).collect();
    let mut out: Vec<Vec<(&'static str, SyllableStress)>> = Vec::with_capacity(n);
    for k in 0..n {
        let start = if k == 0 {
            0
        } else {
            syllable_boundary(phonemes, chosen_vowels[k - 1], chosen_vowels[k])
        };
        let end = if k == n - 1 {
            phonemes.len()
        } else {
            syllable_boundary(phonemes, chosen_vowels[k], chosen_vowels[k + 1])
        };
        out.push(phonemes[start..end].to_vec());
    }
    out
}

/// Where to cut the consonant run between two vowel nuclei.
///
/// Maximal onset, *constrained by English phonotactics*. Handing the
/// whole run to the next syllable's onset — the old rule — is right for a
/// single consonant (`hou·ses` → `hh aw / s ax z`) but wrong the moment
/// there is a cluster: `function` came out as `f ah / ng k sh ax n`,
/// piling five phonemes onto the second note while the first sang a bare
/// `f ah`. `ng k sh` is not a syllable onset any English speaker produces,
/// and the lopsided split is exactly the crammed-note case that destroys
/// intelligibility.
///
/// So: take the longest *legal* onset (at most two phones, checked
/// against [`is_legal_onset_pair`]) and leave the rest as the previous
/// syllable's coda — `func·tion` → `f ah ng k / sh ax n`.
fn syllable_boundary(
    phonemes: &[(&'static str, SyllableStress)],
    prev_vowel: usize,
    next_vowel: usize,
) -> usize {
    let run: Vec<usize> = ((prev_vowel + 1)..next_vowel)
        .filter(|&i| is_consonant(phonemes[i].0))
        .collect();
    match run.len() {
        // Nothing between the nuclei (or only non-consonants): the next
        // syllable starts at its own vowel.
        0 => next_vowel,
        // A single consonant always becomes the next onset.
        1 => run[0],
        _ => {
            let last = run[run.len() - 1];
            let second_last = run[run.len() - 2];
            if is_legal_onset_pair(phonemes[second_last].0, phonemes[last].0) {
                second_last
            } else {
                last
            }
        }
    }
}

/// Is `(a, b)` a consonant pair English allows at the start of a
/// syllable? Covers the productive clusters: stop/fricative + liquid or
/// glide, and `s` + a voiceless stop / nasal / liquid / glide. Anything
/// else (`k sh`, `ng k`, `l t`, …) only ever occurs across a syllable
/// boundary, so the pair is split.
fn is_legal_onset_pair(a: &str, b: &str) -> bool {
    match (a, b) {
        // s-clusters: "spin", "still", "sky", "smile", "snow", "slow",
        // "sweet", "sue".
        ("s", "p" | "t" | "k" | "m" | "n" | "l" | "w" | "y") => true,
        // Obstruent + /r/: "pray", "brew", "tree", "dry", "cry", "grow",
        // "free", "three", "shrink".
        ("p" | "b" | "t" | "d" | "k" | "g" | "f" | "th" | "sh", "r") => true,
        // Obstruent + /l/: "play", "blue", "clay", "glow", "flow".
        // (`s l` is already covered by the s-cluster arm.)
        ("p" | "b" | "k" | "g" | "f", "l") => true,
        // Obstruent + /w/: "twin", "dwell", "quick", "Gwen", "thwart",
        // "what".
        ("t" | "d" | "k" | "g" | "th" | "hh", "w") => true,
        // Obstruent + /y/: "pure", "beauty", "cute", "few", "view",
        // "music", "hue".
        ("p" | "b" | "k" | "f" | "v" | "m" | "hh" | "n", "y") => true,
        _ => false,
    }
}
