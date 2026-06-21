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

use std::collections::HashMap;
use std::sync::OnceLock;

use cmudict_fast::{Cmudict, Stress, Symbol};

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

/// The full English ARPAbet inventory the G2P emits, in canonical order
/// (vowels first, then [`CONSONANTS`]). Silence markers (`AP`/`SP`) and
/// the `cl` closure are *not* listed — those are pipeline control tokens,
/// not lexical phones. Downstream voicebank accessors use this as the
/// universe of singable phones when deciding which symbols a given bank
/// can sing. Every entry round-trips through [`canonical_phoneme`]; the
/// `arpabet_phonemes_are_canonical` test pins the split against
/// [`CONSONANTS`].
pub const ARPABET_PHONEMES: &[&str] = &[
    "aa", "ae", "ah", "ax", "ao", "aw", "ay", "eh", "er", "ey", "ih", "iy", "ow", "oy", "uh", "uw",
    "b", "ch", "d", "dh", "f", "g", "hh", "jh", "k", "l", "m", "n", "ng", "p", "r", "s", "sh", "t",
    "th", "v", "w", "y", "z", "zh",
];

/// Lexical stress level for a syllable, drawn from the CMU dict's stress
/// marks on its vowel(s). The SVS pipeline maps this to per-syllable
/// velocity / tension bumps so primary-stress syllables sing louder &
/// brighter than the function-word schwas around them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum SyllableStress {
    /// Unstressed (CMU `0`) — the schwa-y function-word baseline.
    #[default]
    None,
    /// Secondary stress (CMU `2`) — the weaker stressed syllable in
    /// longer words, e.g. the `un-` in `university`.
    Secondary,
    /// Primary stress (CMU `1`) — the loudest syllable in a word.
    Primary,
}

impl SyllableStress {
    /// Multiplier applied to a note's MIDI velocity when this syllable
    /// is sung. Primary stress boosts ~15 %, secondary ~5 %, none trims
    /// ~10 %. Multiplicative so a quiet phrase still has stress
    /// contrast but doesn't blow the velocity past 1.0.
    pub fn velocity_factor(self) -> f32 {
        match self {
            SyllableStress::Primary => 1.15,
            SyllableStress::Secondary => 1.05,
            SyllableStress::None => 0.90,
        }
    }

    /// Single-character label for compact UI tooltips / debug strings.
    pub fn glyph(self) -> char {
        match self {
            SyllableStress::Primary => '1',
            SyllableStress::Secondary => '2',
            SyllableStress::None => '0',
        }
    }
}

/// Transcribe a whole word to ARPAbet-lowercase phonemes, picking CMU
/// pronunciation variant `variant_idx` (1-indexed: 1 = first / default,
/// 2 = second, ...).
/// CMU lists multiple pronunciations for ambiguous words: e.g. `read`
/// has /rɛd/ (past) at index 1 and /riːd/ (present) at index 2; `live`
/// has the adjective /laɪv/ at 1 and the verb /lɪv/ at 2. Out-of-range
/// indices clamp to the last available variant.
///
/// Returns `(phoneme, stress)` pairs. Stress is only meaningful on
/// vowels — consonants always carry `SyllableStress::None`. Rule-based
/// fallback never knows stress and returns `None` for everything.
fn word_to_phonemes_variant(word: &str, variant_idx: usize) -> Vec<(&'static str, SyllableStress)> {
    let cleaned: String = word
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphabetic() || *c == '\'')
        .collect();
    if cleaned.is_empty() {
        return vec![("ah", SyllableStress::None)];
    }

    if let Some(rules) = dict().get(&cleaned) {
        let pick_idx = variant_idx.saturating_sub(1).min(rules.len().saturating_sub(1));
        if let Some(rule) = rules.get(pick_idx) {
            let out: Vec<(&'static str, SyllableStress)> =
                rule.pronunciation().iter().map(symbol_to_str).collect();
            if !out.is_empty() {
                return ensure_vowel(out);
            }
        }
    }

    // Fallback: letter-pattern rules. Won't match CMU's accuracy but
    // produces something pronounceable for names, made-up words, etc.
    let fallback: Vec<(&'static str, SyllableStress)> = rule_based(&cleaned)
        .into_iter()
        .map(|p| (p, SyllableStress::None))
        .collect();
    ensure_vowel(fallback)
}

/// How many CMU pronunciation variants the dict has for `word` (≥ 1
/// always; OOV words return 1). Lets the UI expose the available
/// alternates so a user knows whether `read(2)` is meaningful for a
/// given word.
pub fn cmu_variant_count(word: &str) -> usize {
    let cleaned: String = word
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphabetic() || *c == '\'')
        .collect();
    if cleaned.is_empty() {
        return 1;
    }
    dict()
        .get(&cleaned)
        .map(|r| r.len().max(1))
        .unwrap_or(1)
}

/// One CMU pronunciation variant of a word, surfaced for the phoneme
/// strip's variant picker so a user can choose between e.g. `read`
/// /riːd/ (present) and /rɛd/ (past).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PronunciationVariant {
    /// 1-indexed variant number — the same value you'd type as the
    /// `(N)` lyric hint (`read(2)` picks index 2).
    pub index: usize,
    /// Phonemes in the lowercase ARPAbet form the SVS pipeline sings,
    /// with per-phoneme lexical stress (meaningful on vowels only).
    /// Identical to what `word(index)` produces in a lyric.
    pub phonemes: Vec<(&'static str, SyllableStress)>,
    /// Short human label: uppercase ARPAbet with CMU stress digits on
    /// the vowels, e.g. `"R IY1 D"`. What you'd write on a score to tell
    /// one variant apart from another.
    pub label: String,
}

/// Enumerate every CMU pronunciation variant of `word`, in CMU order
/// (index 1 = the default). Each entry carries the phoneme sequence the
/// SVS pipeline would sing plus a short uppercase-ARPAbet label for the
/// picker. Out-of-vocabulary words return a single rule-based variant.
///
/// Builds on [`cmu_variant_count`] + the internal variant transcriber,
/// so the phonemes match exactly what `word(N)` produces in a lyric.
/// Always returns at least one variant for any input.
pub fn cmu_variants(word: &str) -> Vec<PronunciationVariant> {
    let count = cmu_variant_count(word);
    (1..=count)
        .map(|index| {
            let phonemes = word_to_phonemes_variant(word, index);
            let label = arpabet_label(&phonemes);
            PronunciationVariant {
                index,
                phonemes,
                label,
            }
        })
        .collect()
}

/// Render a phoneme+stress sequence as an uppercase-ARPAbet display
/// string with CMU stress digits on the vowels:
/// `[("r",None),("iy",Primary),("d",None)]` → `"R IY1 D"`. Consonants
/// and the silence markers (`AP`/`SP`) carry no digit.
fn arpabet_label(phonemes: &[(&'static str, SyllableStress)]) -> String {
    phonemes
        .iter()
        .map(|(p, stress)| {
            let upper = p.to_uppercase();
            if is_consonant(p) || *p == "AP" || *p == "SP" {
                upper
            } else {
                format!("{upper}{}", stress.glyph())
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Map a CMU `Symbol` to the lowercase ARPAbet phoneme string the
/// SVS acoustic model expects, plus its lexical stress. Unstressed AH
/// is special: in English it's the schwa /ə/, and our voicebanks all
/// expose a distinct `ax` symbol for it. Emitting `ax` instead of `ah`
/// for `AH(Stress::None)` makes function words like "the", "about",
/// "another" sound natural instead of overly stressed.
///
/// Consonants always carry `SyllableStress::None` — stress is a
/// property of the vowel nucleus, not the surrounding consonants.
fn symbol_to_str(sym: &Symbol) -> (&'static str, SyllableStress) {
    let map_stress = |s: &Stress| -> SyllableStress {
        match s {
            Stress::None => SyllableStress::None,
            Stress::Primary => SyllableStress::Primary,
            Stress::Secondary => SyllableStress::Secondary,
        }
    };
    match sym {
        Symbol::AA(s) => ("aa", map_stress(s)),
        Symbol::AE(s) => ("ae", map_stress(s)),
        Symbol::AH(Stress::None) => ("ax", SyllableStress::None),
        Symbol::AH(s) => ("ah", map_stress(s)),
        Symbol::AO(s) => ("ao", map_stress(s)),
        Symbol::AW(s) => ("aw", map_stress(s)),
        Symbol::AY(s) => ("ay", map_stress(s)),
        Symbol::B => ("b", SyllableStress::None),
        Symbol::CH => ("ch", SyllableStress::None),
        Symbol::D => ("d", SyllableStress::None),
        Symbol::DH => ("dh", SyllableStress::None),
        Symbol::EH(s) => ("eh", map_stress(s)),
        Symbol::ER(s) => ("er", map_stress(s)),
        Symbol::EY(s) => ("ey", map_stress(s)),
        Symbol::F => ("f", SyllableStress::None),
        Symbol::G => ("g", SyllableStress::None),
        Symbol::HH => ("hh", SyllableStress::None),
        Symbol::IH(s) => ("ih", map_stress(s)),
        Symbol::IY(s) => ("iy", map_stress(s)),
        Symbol::JH => ("jh", SyllableStress::None),
        Symbol::K => ("k", SyllableStress::None),
        Symbol::L => ("l", SyllableStress::None),
        Symbol::M => ("m", SyllableStress::None),
        Symbol::N => ("n", SyllableStress::None),
        Symbol::NG => ("ng", SyllableStress::None),
        Symbol::OW(s) => ("ow", map_stress(s)),
        Symbol::OY(s) => ("oy", map_stress(s)),
        Symbol::P => ("p", SyllableStress::None),
        Symbol::R => ("r", SyllableStress::None),
        Symbol::S => ("s", SyllableStress::None),
        Symbol::SH => ("sh", SyllableStress::None),
        Symbol::T => ("t", SyllableStress::None),
        Symbol::TH => ("th", SyllableStress::None),
        Symbol::UH(s) => ("uh", map_stress(s)),
        Symbol::UW(s) => ("uw", map_stress(s)),
        Symbol::V => ("v", SyllableStress::None),
        Symbol::W => ("w", SyllableStress::None),
        Symbol::Y => ("y", SyllableStress::None),
        Symbol::Z => ("z", SyllableStress::None),
        Symbol::ZH => ("zh", SyllableStress::None),
    }
}

/// Ensure the output contains at least one vowel — the acoustic model
/// can't sing a pure-consonant cluster. Inject an unstressed schwa
/// before the final consonant so `"k l"` becomes `"k ah l"` (the way
/// English speakers actually say "kle").
fn ensure_vowel(
    mut out: Vec<(&'static str, SyllableStress)>,
) -> Vec<(&'static str, SyllableStress)> {
    if !out.iter().any(|(p, _)| !is_consonant(p)) {
        if out.len() >= 2 {
            let insert_at = out.len() - 1;
            out.insert(insert_at, ("ah", SyllableStress::None));
        } else {
            out.push(("ah", SyllableStress::None));
        }
    }
    // Dedup consecutive identical phonemes — doubled consonants in
    // English spelling ("glass", "letter") are single phonemes. The
    // dedup keeps the first occurrence's stress.
    let mut deduped: Vec<(&'static str, SyllableStress)> = Vec::with_capacity(out.len());
    for entry in out {
        if deduped.last().map(|(p, _)| *p) != Some(entry.0) {
            deduped.push(entry);
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

/// Each lyric token resolved out of the draft. Either a normal English
/// word (CMU-lookup + split) or an explicit phoneme block the user
/// typed between square brackets (`[hh ah l ow]`) to override
/// pronunciation for proper nouns, foreign words, or anything CMU
/// gets wrong. Internal to `phonemes_for_draft`'s tokenizer.
enum LyricToken {
    Word { cleaned: String, syl_count: usize, variant_idx: usize },
    /// Pre-segmented phoneme groups, one inner vec per syllable.
    /// User-typed overrides carry no stress, so the syllable stress
    /// defaults to `None`.
    PhonemeBlock(Vec<Vec<&'static str>>),
}

/// Walk a line and split it into `LyricToken`s. Recognises `[...]`
/// blocks as inline phoneme overrides; everything else is a word.
fn tokenize_line(text: &str) -> Vec<LyricToken> {
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

/// The full ARPAbet symbol inventory the SVS pipeline understands, in a
/// stable display order: vowels (incl. the schwa `ax`), then consonants,
/// then the silence markers `AP`/`SP`. Drives the add-phoneme palette so
/// callers can render a button per symbol without poking at the internal
/// `phf` map. Kept in lockstep with [`ARPABET_INVENTORY`] by a test.
pub const ARPABET_SYMBOLS: &[&str] = &[
    // Vowels.
    "aa", "ae", "ah", "ax", "ao", "aw", "ay", "eh", "er", "ey", "ih", "iy", "ow", "oy", "uh",
    "uw",
    // Consonants.
    "b", "ch", "d", "dh", "f", "g", "hh", "jh", "k", "l", "m", "n", "ng", "p", "r", "s", "sh",
    "t", "th", "v", "w", "y", "z", "zh",
    // Silence markers — sung as a rest / breath, not stored in the phf map.
    "AP", "SP",
];

/// ARPAbet phoneme inventory the SVS pipeline understands. Keys are the
/// lowercase canonical forms; the value is the same `&'static str` so we
/// can hand it back as the canonical form after a case-insensitive
/// lookup. `phf_set` would be nicer, but `phf::Set::get_key` returns
/// `&&'static str` which is awkward to thread through callers — a
/// self-mapping `phf::Map` gives us a clean `Option<&'static str>`.
static ARPABET_INVENTORY: phf::Map<&'static str, &'static str> = phf::phf_map! {
    "aa" => "aa",
    "ae" => "ae",
    "ah" => "ah",
    "ax" => "ax",
    "ao" => "ao",
    "aw" => "aw",
    "ay" => "ay",
    "eh" => "eh",
    "er" => "er",
    "ey" => "ey",
    "ih" => "ih",
    "iy" => "iy",
    "ow" => "ow",
    "oy" => "oy",
    "uh" => "uh",
    "uw" => "uw",
    "b" => "b",
    "ch" => "ch",
    "d" => "d",
    "dh" => "dh",
    "f" => "f",
    "g" => "g",
    "hh" => "hh",
    "jh" => "jh",
    "k" => "k",
    "l" => "l",
    "m" => "m",
    "n" => "n",
    "ng" => "ng",
    "p" => "p",
    "r" => "r",
    "s" => "s",
    "sh" => "sh",
    "t" => "t",
    "th" => "th",
    "v" => "v",
    "w" => "w",
    "y" => "y",
    "z" => "z",
    "zh" => "zh",
};

/// Validate a user-typed phoneme symbol against the ARPAbet inventory
/// the SVS pipeline understands, returning the canonical `&'static str`
/// form. Accepts case-insensitive input and the silence markers
/// (`AP`, `SP`). Returns `None` for unknown symbols so callers can
/// silently drop typos rather than crash. Public so the phoneme-strip
/// editor and add-phoneme palette can validate user input against the
/// exact same inventory the SVS pipeline sings.
///
/// Returns `Some(sym)` for every entry in [`ARPABET_PHONEMES`] plus the
/// `AP`/`SP` silence markers — the canonical universe voicebank
/// accessors validate phoneme overrides against.
pub fn canonical_phoneme(sym: &str) -> Option<&'static str> {
    // Silence markers are uppercase-only by convention; check the
    // original input before lowercasing so `"ap"` / `"sp"` don't sneak
    // through as silence.
    if sym == "AP" {
        return Some("AP");
    }
    if sym == "SP" {
        return Some("SP");
    }
    ARPABET_INVENTORY.get(sym.to_lowercase().as_str()).copied()
}

/// Resolve a draft into one phoneme list per syllable. For each
/// syllable in the draft we look up the *whole word* it belongs to in
/// CMU, then slice the resulting phoneme stream across the word's
/// syllables. This matches how the SVS model expects phonemes to land
/// on note boundaries when one word spans multiple notes (e.g.
/// `hou·ses` → note 1 gets `[hh aw z]`, note 2 gets `[ah z]`).
///
/// Power-user escape hatch: `[hh ah l ow]` in the lyric is taken
/// verbatim as phonemes for one syllable, bypassing CMU. Use
/// `[l ih · l iy · ah]` (or `[l ih]·[l iy]·[ah]`) for multi-syllable
/// overrides. Helpful for proper nouns and foreign-language words
/// where the CMU dict or rule-based fallback misfire.
///
/// Returns one `Vec<&str>` per output syllable. Use `resolve_draft`
/// when you also need stress / surface-label information.
pub fn phonemes_for_draft(draft: &[crate::derive::LyricLine]) -> Vec<Vec<&'static str>> {
    resolve_draft(draft)
        .into_iter()
        .map(|s| s.phonemes)
        .collect()
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

/// Where a syllable's phonemes came from, so the UI can badge edited /
/// dictionary syllables and downstream code can reason about how much to
/// trust the transcription. The resolution precedence is
/// `Edited` > `Dict` > `Auto`: a per-syllable override (or an inline
/// `[..]` block) beats a caller-supplied dictionary hit, which beats the
/// CMU / rule-based auto transcription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PhonemeProvenance {
    /// CMU dictionary or rule-based fallback — the default path.
    #[default]
    Auto,
    /// A caller-supplied word→phonemes dictionary entry replaced CMU.
    Dict,
    /// A per-syllable phoneme override: an inline `[..]` lyric block or
    /// a caller-supplied per-syllable edit.
    Edited,
}

/// A caller-supplied pronunciation dictionary: cleaned lowercase word →
/// the flat phoneme list to sing for the *whole* word, overriding CMU.
/// Build the phoneme vec with [`canonical_phoneme`] so every symbol is a
/// valid `&'static str` the pipeline recognises. List phonemes flat (no
/// `·`); the resolver re-splits them across the word's syllable count
/// exactly like the CMU path. Dictionary phonemes carry no stress.
pub type PhonemeDictionary = HashMap<String, Vec<&'static str>>;

/// A caller-supplied per-syllable phoneme override, keyed by the
/// resolved-syllable index (the `syllable_index` an [`AssignedSyllable`]
/// reports). Highest precedence — replaces whatever the resolver picked
/// for that syllable. Build values with [`canonical_phoneme`].
pub type SyllableOverrides = HashMap<usize, Vec<&'static str>>;

/// One syllable resolved against the lyric draft. Carries the surface
/// label (the glyphs you'd write on a score), the phoneme list the
/// SVS model will sing, a `is_word_end` flag that drives SP injection
/// between words, the syllable's lexical stress (drawn from CMU's
/// stress marks on its vowel), and the phonemes' [`PhonemeProvenance`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSyllable {
    pub label: String,
    pub phonemes: Vec<&'static str>,
    pub is_word_end: bool,
    pub stress: SyllableStress,
    pub provenance: PhonemeProvenance,
}

/// One note's assignment after the lyric side-table annotations have
/// been applied to the resolved draft. The cursor in
/// [`assign_syllables_to_notes`] produces a `Vec<AssignedSyllable>`
/// of exactly `note_count` entries — the single source of truth
/// shared between the vocal roll (lyrics on notes + phoneme strip)
/// and the SVS pipeline (`build_segment`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignedSyllable {
    /// Glyphs to draw on the note body or in the phoneme strip's
    /// label column. For slurs this is `"+"`. For overrides it's the
    /// user-typed string; otherwise the resolved-syllable surface.
    pub label: String,
    /// Phoneme list the SVS pipeline sings for this note. For slurs
    /// this is the held vowel from the previous syllable (single
    /// element vec).
    pub phonemes: Vec<&'static str>,
    pub is_slur: bool,
    /// `true` when the underlying resolved syllable was the last in
    /// its word *and* this note is non-slur. Slur notes never sit on
    /// a word boundary by definition.
    pub is_word_end: bool,
    /// Which resolved-syllable index this note maps to. Slur notes
    /// inherit the previous non-slur note's index. Out-of-range when
    /// the draft has fewer syllables than non-slur notes.
    pub syllable_index: usize,
    /// Lexical stress for this syllable, drawn from CMU. Slur notes
    /// inherit the held syllable's stress. Drives the SVS pipeline's
    /// per-syllable velocity / tension bump and the stress overlay
    /// in the vocal roll.
    pub stress: SyllableStress,
    /// Where this note's phonemes came from. Slur notes inherit the
    /// held syllable's provenance. See [`PhonemeProvenance`].
    pub provenance: PhonemeProvenance,
}

/// Resolve every syllable in a lyric draft to its (surface, phonemes,
/// word-end) tuple. One pass through `tokenize_line` — guarantees the
/// surface labels and phoneme groups stay in lockstep, so a per-
/// syllable assertion like `labels.len() == phonemes.len()` becomes a
/// property of the type rather than a discipline.
///
/// Phoneme-block tokens (`[hh ah]` overrides) get their label set to
/// the bracketed phoneme list — the user explicitly typed phonemes,
/// not glyphs, so that's the most faithful surface to display.
///
/// Equivalent to [`resolve_draft_with_dict`] with an empty dictionary.
pub fn resolve_draft(draft: &[crate::derive::LyricLine]) -> Vec<ResolvedSyllable> {
    resolve_draft_with_dict(draft, &PhonemeDictionary::new())
}

/// Like [`resolve_draft`], but a caller-supplied [`PhonemeDictionary`]
/// takes precedence over the CMU / rule-based transcription for any word
/// it contains (matched on the cleaned, lowercased word). Dictionary
/// phonemes are re-split across the word's syllable count just like CMU
/// output and reported with [`PhonemeProvenance::Dict`]; words absent
/// from the dictionary resolve exactly as before (`Auto`). Inline
/// `[..]` blocks always win and report `Edited`.
pub fn resolve_draft_with_dict(
    draft: &[crate::derive::LyricLine],
    dictionary: &PhonemeDictionary,
) -> Vec<ResolvedSyllable> {
    let mut tokens: Vec<LyricToken> = Vec::new();
    for line in draft {
        tokens.extend(tokenize_line(&line.text));
    }
    let mut out: Vec<ResolvedSyllable> = Vec::new();
    for token in tokens {
        match token {
            LyricToken::PhonemeBlock(groups) => {
                let last_idx = groups.len().saturating_sub(1);
                for (i, g) in groups.into_iter().enumerate() {
                    out.push(ResolvedSyllable {
                        label: format!("[{}]", g.join(" ")),
                        phonemes: g,
                        is_word_end: i == last_idx,
                        // Bracket overrides carry no stress info.
                        stress: SyllableStress::None,
                        // An inline block is the user typing phonemes
                        // directly — the highest-precedence source.
                        provenance: PhonemeProvenance::Edited,
                    });
                }
            }
            LyricToken::Word { cleaned, syl_count, variant_idx } => {
                let (phonemes, provenance) = match dictionary.get(&cleaned) {
                    Some(dict_phonemes) => (
                        dict_phonemes
                            .iter()
                            .map(|p| (*p, SyllableStress::None))
                            .collect::<Vec<_>>(),
                        PhonemeProvenance::Dict,
                    ),
                    None => (
                        word_to_phonemes_variant(&cleaned, variant_idx),
                        PhonemeProvenance::Auto,
                    ),
                };
                let phoneme_groups: Vec<Vec<(&'static str, SyllableStress)>> = if syl_count <= 1 {
                    vec![phonemes]
                } else {
                    split_into_syllables(&phonemes, syl_count)
                };
                // Surface labels via `syllabify_word`, which inserts
                // `·` markers using the same CMU syllable count the
                // phoneme split uses. Falling back to the cleaned word
                // when syllabify-word can't reach the target keeps the
                // two slices balanced.
                let with_dots = syllabify_word(&cleaned, syl_count);
                let labels: Vec<String> = with_dots
                    .split('\u{00B7}')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let n = phoneme_groups.len();
                let last_idx = n.saturating_sub(1);
                for i in 0..n {
                    let label = labels.get(i).cloned().unwrap_or_default();
                    let group = phoneme_groups.get(i).cloned().unwrap_or_default();
                    let stress = group
                        .iter()
                        .filter(|(p, _)| !is_consonant(p))
                        .map(|(_, s)| *s)
                        .max()
                        .unwrap_or(SyllableStress::None);
                    let phonemes: Vec<&'static str> = group.into_iter().map(|(p, _)| p).collect();
                    out.push(ResolvedSyllable {
                        label,
                        phonemes,
                        is_word_end: i == last_idx,
                        stress,
                        provenance,
                    });
                }
            }
        }
    }
    out
}

/// Map a per-note annotation vec to per-note `AssignedSyllable`s,
/// walking a cursor through `syllables` and skipping it on slur
/// notes. The single source of truth for the cursor model — every
/// view + the SVS pipeline use this.
///
/// `annotations[i]` is interpreted as:
///
/// * `""` (empty)  — consume the next resolved syllable.
/// * `"+"` / `"-"` — slur: inherit the previous note's vowel, no
///   cursor advance.
/// * anything else — explicit label override (cursor still advances;
///   phonemes still come from the resolved syllable).
///
/// Returns exactly `note_count` entries.
///
/// Equivalent to [`assign_syllables_to_notes_with`] with no per-syllable
/// overrides.
pub fn assign_syllables_to_notes(
    syllables: &[ResolvedSyllable],
    annotations: &[String],
    note_count: usize,
) -> Vec<AssignedSyllable> {
    assign_syllables_to_notes_with(syllables, annotations, note_count, &SyllableOverrides::new())
}

/// Like [`assign_syllables_to_notes`], but applies caller-supplied
/// per-syllable phoneme [`SyllableOverrides`] — the highest-precedence
/// resolution layer. When a non-slur note resolves to a syllable whose
/// index is present in `overrides`, that note sings the override
/// phonemes and reports [`PhonemeProvenance::Edited`]; otherwise it
/// keeps the resolved syllable's phonemes and provenance (`Dict` if the
/// syllable came from a dictionary, else `Auto`). A following slur note
/// holds the (possibly overridden) vowel and inherits its provenance.
///
/// With an empty `overrides` map this is byte-for-byte identical to the
/// pre-override behaviour, so existing call sites are unaffected.
pub fn assign_syllables_to_notes_with(
    syllables: &[ResolvedSyllable],
    annotations: &[String],
    note_count: usize,
    overrides: &SyllableOverrides,
) -> Vec<AssignedSyllable> {
    let mut out: Vec<AssignedSyllable> = Vec::with_capacity(note_count);
    let mut cursor: usize = 0;
    let mut last_syllable_idx: usize = 0;
    let mut last_vowel: Option<&'static str> = None;
    let mut last_stress: SyllableStress = SyllableStress::None;
    let mut last_provenance: PhonemeProvenance = PhonemeProvenance::Auto;
    for i in 0..note_count {
        let entry = annotations.get(i).map(|s| s.trim()).unwrap_or("");
        if is_slur_lyric(entry) {
            let phonemes: Vec<&'static str> =
                last_vowel.map(|v| vec![v]).unwrap_or_default();
            out.push(AssignedSyllable {
                label: SLUR_MARKER.to_string(),
                phonemes,
                is_slur: true,
                is_word_end: false,
                syllable_index: last_syllable_idx,
                stress: last_stress,
                provenance: last_provenance,
            });
            continue;
        }
        let syl_opt = syllables.get(cursor);
        let syl_index = cursor;
        cursor += 1;
        let Some(syl) = syl_opt else {
            out.push(AssignedSyllable {
                label: String::new(),
                phonemes: Vec::new(),
                is_slur: false,
                is_word_end: false,
                syllable_index: syl_index,
                stress: SyllableStress::None,
                provenance: PhonemeProvenance::Auto,
            });
            continue;
        };
        // Override > dictionary > CMU-auto: a per-syllable override (keyed
        // by resolved-syllable index) replaces the phonemes and stamps
        // `Edited`; otherwise we keep the syllable's own phonemes and
        // provenance.
        let (phonemes, provenance) = match overrides.get(&syl_index) {
            Some(ov) => (ov.clone(), PhonemeProvenance::Edited),
            None => (syl.phonemes.clone(), syl.provenance),
        };
        // Cache the last non-consonant phoneme so a following slur
        // note can hold the vowel. Falls back to the final phoneme
        // when the syllable is all-consonant (rare; only happens for
        // pathological overrides). Reads the resolved phonemes (after
        // any override) so a slur holds the edited vowel.
        if let Some(v) = phonemes.iter().rev().find(|p| !is_consonant(p)) {
            last_vowel = Some(*v);
        } else if let Some(v) = phonemes.last() {
            last_vowel = Some(*v);
        }
        last_syllable_idx = syl_index;
        last_stress = syl.stress;
        last_provenance = provenance;
        let label = if !entry.is_empty() {
            entry.to_string()
        } else {
            syl.label.clone()
        };
        out.push(AssignedSyllable {
            label,
            phonemes,
            is_slur: false,
            is_word_end: syl.is_word_end,
            syllable_index: syl_index,
            stress: syl.stress,
            provenance,
        });
    }
    out
}


/// Split a phoneme list into `n` syllable-shaped chunks. Tries to
/// give each chunk exactly one vowel; consonants between vowels go
/// to the chunk *after* (onset of the next syllable) for English-like
/// resyllabification (`hou·ses` → `hh aw / z ah z`). Operates on
/// `(phoneme, stress)` pairs so the stress on each vowel travels with
/// the chunk it ends up in.
fn split_into_syllables(
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
