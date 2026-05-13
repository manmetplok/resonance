//! Grapheme-to-phoneme for English lyrics. Returns phonemes in the
//! ARPAbet-lowercase form the DiffSinger TIGER acoustic model expects
//! (`aa`, `ae`, `ah`, `b`, `ch`, …, `zh`).
//!
//! Implementation strategy:
//!   1. Look up the whole word in the bundled CMU Pronouncing
//!      Dictionary (≈135 k English words, public domain, vendored
//!      under `data/cmudict.dict`). This handles ~95 % of real English
//!      text correctly — including all the weird cases the rule-based
//!      transcriber gets wrong (`houses` → `hh aw z ah z`, `break` →
//!      `b r ey k`, `the` → `dh ah`, `light` → `l ay t`).
//!   2. If the word isn't in the dictionary (names, made-up words,
//!      typos), fall back to letter-pattern rules.
//!
//! The dictionary is loaded once at first call via `OnceLock` so the
//! parse cost (~50 ms on first lookup) is amortised across an entire
//! song.

use std::sync::OnceLock;

use cmudict_fast::{Cmudict, Symbol};

/// Embedded CMU Pronouncing Dictionary v0.7b. Licensed under the
/// permissive CMUDict license (see `data/LICENSE-CMUDICT`). ~3.7 MB
/// of raw text — adds ~3 MB to the release binary.
const CMUDICT_TEXT: &str = include_str!("../data/cmudict.dict");

fn dict() -> &'static Cmudict {
    static DICT: OnceLock<Cmudict> = OnceLock::new();
    DICT.get_or_init(|| {
        CMUDICT_TEXT
            .parse::<Cmudict>()
            .expect("bundled cmudict parses")
    })
}

/// Phoneme symbols treated as consonants for duration sharing in the
/// SVS pipeline.
pub const CONSONANTS: &[&str] = &[
    "b", "ch", "d", "dh", "f", "g", "hh", "jh", "k", "l", "m", "n", "ng", "p", "r", "s", "sh",
    "t", "th", "v", "w", "y", "z", "zh",
];

pub fn is_consonant(ph: &str) -> bool {
    CONSONANTS.contains(&ph)
}

/// Transcribe a whole word to ARPAbet-lowercase phonemes. Tries the
/// CMU dict first; falls back to letter-pattern rules for unknown
/// words (names, made-up words, typos). Always emits at least one
/// vowel so the acoustic model has something to sing.
fn word_to_phonemes(word: &str) -> Vec<&'static str> {
    let cleaned: String = word
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphabetic() || *c == '\'')
        .collect();
    if cleaned.is_empty() {
        return vec!["ah"];
    }

    if let Some(rules) = dict().get(&cleaned) {
        if let Some(rule) = rules.first() {
            let out: Vec<&'static str> = rule
                .pronunciation()
                .iter()
                .map(symbol_to_str)
                .collect();
            if !out.is_empty() {
                return ensure_vowel(out);
            }
        }
    }

    // Fallback: letter-pattern rules. Won't match CMU's accuracy but
    // produces something pronounceable for names, made-up words, etc.
    ensure_vowel(rule_based(&cleaned))
}

/// Map a CMU `Symbol` to the lowercase ARPAbet phoneme string the
/// TIGER acoustic model expects. Drops stress information since the
/// model doesn't have separate stress-bearing variants.
fn symbol_to_str(sym: &Symbol) -> &'static str {
    match sym {
        Symbol::AA(_) => "aa",
        Symbol::AE(_) => "ae",
        Symbol::AH(_) => "ah",
        Symbol::AO(_) => "ao",
        Symbol::AW(_) => "aw",
        Symbol::AY(_) => "ay",
        Symbol::B => "b",
        Symbol::CH => "ch",
        Symbol::D => "d",
        Symbol::DH => "dh",
        Symbol::EH(_) => "eh",
        Symbol::ER(_) => "er",
        Symbol::EY(_) => "ey",
        Symbol::F => "f",
        Symbol::G => "g",
        Symbol::HH => "hh",
        Symbol::IH(_) => "ih",
        Symbol::IY(_) => "iy",
        Symbol::JH => "jh",
        Symbol::K => "k",
        Symbol::L => "l",
        Symbol::M => "m",
        Symbol::N => "n",
        Symbol::NG => "ng",
        Symbol::OW(_) => "ow",
        Symbol::OY(_) => "oy",
        Symbol::P => "p",
        Symbol::R => "r",
        Symbol::S => "s",
        Symbol::SH => "sh",
        Symbol::T => "t",
        Symbol::TH => "th",
        Symbol::UH(_) => "uh",
        Symbol::UW(_) => "uw",
        Symbol::V => "v",
        Symbol::W => "w",
        Symbol::Y => "y",
        Symbol::Z => "z",
        Symbol::ZH => "zh",
    }
}

/// Ensure the output contains at least one vowel — the acoustic model
/// can't sing a pure-consonant cluster. Inject a schwa before the
/// final consonant so `"k l"` becomes `"k ah l"` (the way English
/// speakers actually say "kle").
fn ensure_vowel(mut out: Vec<&'static str>) -> Vec<&'static str> {
    if !out.iter().any(|p| !is_consonant(p)) {
        if out.len() >= 2 {
            let insert_at = out.len() - 1;
            out.insert(insert_at, "ah");
        } else {
            out.push("ah");
        }
    }
    // Dedup consecutive identical phonemes — doubled consonants in
    // English spelling ("glass", "letter") are single phonemes.
    let mut deduped: Vec<&'static str> = Vec::with_capacity(out.len());
    for p in out {
        if deduped.last().copied() != Some(p) {
            deduped.push(p);
        }
    }
    deduped
}

/// Fallback transcriber for words missing from CMU. The rules are the
/// same as the previous `vocal_g2p.rs` implementation — good enough
/// for invented words and proper names that wouldn't be in any
/// pronouncing dictionary anyway.
fn rule_based(word: &str) -> Vec<&'static str> {
    let chars: Vec<char> = word.chars().filter(|c| c.is_alphabetic()).collect();
    let mut out: Vec<&'static str> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let two = if i + 1 < chars.len() {
            Some((chars[i], chars[i + 1]))
        } else {
            None
        };

        if i == chars.len() - 1 && chars[i] == 'e' && i > 0 {
            break;
        }

        match two {
            Some(('c', 'h')) => { out.push("ch"); i += 2; continue; }
            Some(('s', 'h')) => { out.push("sh"); i += 2; continue; }
            Some(('t', 'h')) => { out.push("th"); i += 2; continue; }
            Some(('n', 'g')) => { out.push("ng"); i += 2; continue; }
            Some(('p', 'h')) => { out.push("f"); i += 2; continue; }
            Some(('w', 'h')) => { out.push("w"); i += 2; continue; }
            Some(('q', 'u')) => { out.push("k"); out.push("w"); i += 2; continue; }
            Some(('c', 'k')) => { out.push("k"); i += 2; continue; }
            Some(('g', 'h')) => { i += 2; continue; }
            Some(('a', 'i')) | Some(('a', 'y')) => { out.push("ey"); i += 2; continue; }
            Some(('e', 'a')) | Some(('e', 'e')) | Some(('i', 'e')) => { out.push("iy"); i += 2; continue; }
            Some(('o', 'a')) | Some(('o', 'w')) => { out.push("ow"); i += 2; continue; }
            Some(('o', 'o')) => { out.push("uw"); i += 2; continue; }
            Some(('o', 'u')) => { out.push("aw"); i += 2; continue; }
            Some(('o', 'i')) | Some(('o', 'y')) => { out.push("oy"); i += 2; continue; }
            Some(('a', 'u')) | Some(('a', 'w')) => { out.push("ao"); i += 2; continue; }
            _ => {}
        }

        let p: Option<&'static str> = match chars[i] {
            'a' => Some("ae"), 'b' => Some("b"), 'c' => Some("k"), 'd' => Some("d"),
            'e' => Some("eh"), 'f' => Some("f"), 'g' => Some("g"), 'h' => Some("hh"),
            'i' => Some("ih"), 'j' => Some("jh"), 'k' => Some("k"), 'l' => Some("l"),
            'm' => Some("m"), 'n' => Some("n"), 'o' => Some("ow"), 'p' => Some("p"),
            'q' => Some("k"), 'r' => Some("r"), 's' => Some("s"), 't' => Some("t"),
            'u' => Some("ah"), 'v' => Some("v"), 'w' => Some("w"),
            'x' => { out.push("k"); Some("s") }
            'y' => if out.is_empty() { Some("y") } else { Some("iy") },
            'z' => Some("z"),
            _ => None,
        };
        if let Some(ph) = p {
            out.push(ph);
        }
        i += 1;
    }
    out
}

/// Resolve a draft into one phoneme list per syllable. For each
/// syllable in the draft we look up the *whole word* it belongs to in
/// CMU, then slice the resulting phoneme stream across the word's
/// syllables. This matches how the SVS model expects phonemes to land
/// on note boundaries when one word spans multiple notes (e.g.
/// `hou·ses` → note 1 gets `[hh aw z]`, note 2 gets `[ah z]`).
///
/// Returns one `Vec<&str>` per output syllable.
pub fn phonemes_for_draft(draft: &[crate::derive::LyricLine]) -> Vec<Vec<&'static str>> {
    // Build a list of (word, syllable_count) entries so we can split
    // multi-syllable words proportionally.
    let mut entries: Vec<(String, usize)> = Vec::new();
    for line in draft {
        // Split the line into words first (whitespace), then count
        // syllables per word via `·` separators.
        let stripped: String = line
            .text
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        for word_raw in stripped.split_whitespace() {
            // Strip leading/trailing punctuation but keep `·` inside.
            let word = word_raw.trim_matches(|c: char| {
                !c.is_alphabetic() && c != '\'' && c != '\u{00B7}'
            });
            let syl_count = word.split('\u{00B7}').count().max(1);
            let cleaned: String = word
                .chars()
                .filter(|c| c.is_alphabetic() || *c == '\'')
                .collect();
            if !cleaned.is_empty() {
                entries.push((cleaned, syl_count));
            }
        }
    }

    let mut out: Vec<Vec<&'static str>> = Vec::new();
    for (word, syl_count) in entries {
        let phonemes = word_to_phonemes(&word);
        if syl_count == 1 {
            out.push(phonemes);
            continue;
        }
        // Split the phoneme stream into `syl_count` chunks. Each chunk
        // gets a contiguous slice that starts with a consonant onset
        // (when available) and contains exactly one vowel — that maps
        // naturally to a "syllable" boundary in the trained model.
        let slices = split_into_syllables(&phonemes, syl_count);
        for slice in slices {
            out.push(slice);
        }
    }
    out
}

/// Split a phoneme list into `n` syllable-shaped chunks. Tries to
/// give each chunk exactly one vowel; consonants between vowels go
/// to the chunk *after* (onset of the next syllable) for English-like
/// resyllabification (`hou·ses` → `hh aw / z ah z`).
fn split_into_syllables(phonemes: &[&'static str], n: usize) -> Vec<Vec<&'static str>> {
    if n <= 1 {
        return vec![phonemes.to_vec()];
    }
    // Find vowel positions.
    let vowels: Vec<usize> = phonemes
        .iter()
        .enumerate()
        .filter(|(_, p)| !is_consonant(p))
        .map(|(i, _)| i)
        .collect();
    if vowels.len() < n {
        // Not enough vowels — emit one chunk per requested syllable
        // by spreading the phonemes evenly.
        let mut out = Vec::with_capacity(n);
        let chunk_size = phonemes.len().max(1) / n.max(1);
        for k in 0..n {
            let start = k * chunk_size;
            let end = if k == n - 1 {
                phonemes.len()
            } else {
                (k + 1) * chunk_size
            };
            let chunk: Vec<&'static str> = phonemes[start..end.min(phonemes.len())].to_vec();
            if chunk.is_empty() {
                out.push(vec!["ah"]);
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
    let mut out: Vec<Vec<&'static str>> = Vec::with_capacity(n);
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
                .find(|&i| is_consonant(phonemes[i]))
                .unwrap_or(cur_v)
        };
        let end = if k == n - 1 {
            phonemes.len()
        } else {
            let cur_v = chosen_vowels[k];
            let next_v = chosen_vowels[k + 1];
            ((cur_v + 1)..next_v)
                .find(|&i| is_consonant(phonemes[i]))
                .unwrap_or(next_v)
        };
        out.push(phonemes[start..end].to_vec());
    }
    out
}
