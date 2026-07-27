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
                    // New vowel run — boundary belongs immediately
                    // before the most-recent consonant cluster (we
                    // step backward from i over the preceding
                    // consonants). The split position is the index
                    // *before* the first consonant of that cluster.
                    let mut k = i;
                    while k > 0 && is_letter(chars[k - 1]) && !is_vowel(chars[k - 1]) {
                        k -= 1;
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

/// Insert `·` markers into a whole lyric line so each word matches
/// CMU's syllable count. Words that already have enough dots are left
/// alone (preserving user-intentional melismas with extra dots).
pub fn auto_syllabify_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut first = true;
    for word_raw in text.split_whitespace() {
        if !first {
            out.push(' ');
        }
        first = false;
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

/// Split a phoneme list into `n` syllable-shaped chunks. Tries to
/// give each chunk exactly one vowel; consonants between vowels go
/// to the chunk *after* (onset of the next syllable) for English-like
/// resyllabification (`hou·ses` → `hh aw / z ah z`). Operates on
/// `(phoneme, stress)` pairs so the stress on each vowel travels with
/// the chunk it ends up in.
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
    // nuclei; split between two adjacent vowels by putting all
    // intermediate consonants into the *second* syllable's onset
    // (English bias — "houses" splits as "hou-ses" not "hous-es").
    let chosen_vowels: Vec<usize> = vowels.iter().copied().take(n).collect();
    let mut out: Vec<Vec<(&'static str, SyllableStress)>> = Vec::with_capacity(n);
    for k in 0..n {
        let start = if k == 0 {
            0
        } else {
            // Boundary between vowels k-1 and k: split before the
            // last consonant cluster, so the consonants attach as
            // onset to the new syllable.
            let prev_v = chosen_vowels[k - 1];
            let cur_v = chosen_vowels[k];
            ((prev_v + 1)..cur_v)
                .find(|&i| is_consonant(phonemes[i].0))
                .unwrap_or(cur_v)
        };
        let end = if k == n - 1 {
            phonemes.len()
        } else {
            let cur_v = chosen_vowels[k];
            let next_v = chosen_vowels[k + 1];
            ((cur_v + 1)..next_v)
                .find(|&i| is_consonant(phonemes[i].0))
                .unwrap_or(next_v)
        };
        out.push(phonemes[start..end].to_vec());
    }
    out
}
