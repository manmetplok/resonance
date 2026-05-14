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

/// Transcribe a whole word to ARPAbet-lowercase phonemes. Tries the
/// CMU dict first; falls back to letter-pattern rules for unknown
/// words (names, made-up words, typos). Always emits at least one
/// vowel so the acoustic model has something to sing.
fn word_to_phonemes(word: &str) -> Vec<&'static str> {
    word_to_phonemes_variant(word, 1)
}

/// Like `word_to_phonemes` but picks CMU pronunciation variant
/// `variant_idx` (1-indexed: 1 = first / default, 2 = second, ...).
/// CMU lists multiple pronunciations for ambiguous words: e.g. `read`
/// has /rɛd/ (past) at index 1 and /riːd/ (present) at index 2; `live`
/// has the adjective /laɪv/ at 1 and the verb /lɪv/ at 2. Out-of-range
/// indices clamp to the last available variant.
fn word_to_phonemes_variant(word: &str, variant_idx: usize) -> Vec<&'static str> {
    let cleaned: String = word
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphabetic() || *c == '\'')
        .collect();
    if cleaned.is_empty() {
        return vec!["ah"];
    }

    if let Some(rules) = dict().get(&cleaned) {
        let pick_idx = variant_idx.saturating_sub(1).min(rules.len().saturating_sub(1));
        if let Some(rule) = rules.get(pick_idx) {
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

/// Map a CMU `Symbol` to the lowercase ARPAbet phoneme string the
/// SVS acoustic model expects. Stress digits are mostly dropped (the
/// model doesn't have stressed/unstressed variants for most vowels),
/// but unstressed AH is special: in English it's the schwa /ə/, and
/// our voicebanks all expose a distinct `ax` symbol for it. Emitting
/// `ax` instead of `ah` for `AH(Stress::None)` makes function words
/// like "the", "about", "another" sound natural instead of overly
/// stressed.
fn symbol_to_str(sym: &Symbol) -> &'static str {
    match sym {
        Symbol::AA(_) => "aa",
        Symbol::AE(_) => "ae",
        Symbol::AH(Stress::None) => "ax",
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
    let phonemes = word_to_phonemes(&cleaned);
    phonemes.iter().filter(|p| !is_consonant(p)).count().max(1)
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

/// Validate a user-typed phoneme symbol against the ARPAbet inventory
/// the SVS pipeline understands, returning the canonical `&'static str`
/// form. Accepts case-insensitive input and the silence markers
/// (`AP`, `SP`). Returns `None` for unknown symbols so callers can
/// silently drop typos rather than crash.
fn canonical_phoneme(sym: &str) -> Option<&'static str> {
    let lower = sym.to_lowercase();
    match lower.as_str() {
        "aa" => Some("aa"),
        "ae" => Some("ae"),
        "ah" => Some("ah"),
        "ax" => Some("ax"),
        "ao" => Some("ao"),
        "aw" => Some("aw"),
        "ay" => Some("ay"),
        "eh" => Some("eh"),
        "er" => Some("er"),
        "ey" => Some("ey"),
        "ih" => Some("ih"),
        "iy" => Some("iy"),
        "ow" => Some("ow"),
        "oy" => Some("oy"),
        "uh" => Some("uh"),
        "uw" => Some("uw"),
        "b" => Some("b"),
        "ch" => Some("ch"),
        "d" => Some("d"),
        "dh" => Some("dh"),
        "f" => Some("f"),
        "g" => Some("g"),
        "hh" => Some("hh"),
        "jh" => Some("jh"),
        "k" => Some("k"),
        "l" => Some("l"),
        "m" => Some("m"),
        "n" => Some("n"),
        "ng" => Some("ng"),
        "p" => Some("p"),
        "r" => Some("r"),
        "s" => Some("s"),
        "sh" => Some("sh"),
        "t" => Some("t"),
        "th" => Some("th"),
        "v" => Some("v"),
        "w" => Some("w"),
        "y" => Some("y"),
        "z" => Some("z"),
        "zh" => Some("zh"),
        // Silence markers (uppercase by convention).
        _ if sym == "AP" => Some("AP"),
        _ if sym == "SP" => Some("SP"),
        _ => None,
    }
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
/// Returns one `Vec<&str>` per output syllable.
pub fn phonemes_for_draft(draft: &[crate::derive::LyricLine]) -> Vec<Vec<&'static str>> {
    let mut tokens: Vec<LyricToken> = Vec::new();
    for line in draft {
        tokens.extend(tokenize_line(&line.text));
    }

    let mut out: Vec<Vec<&'static str>> = Vec::new();
    for token in tokens {
        match token {
            LyricToken::PhonemeBlock(groups) => out.extend(groups),
            LyricToken::Word { cleaned, syl_count, variant_idx } => {
                let phonemes = word_to_phonemes_variant(&cleaned, variant_idx);
                if syl_count == 1 {
                    out.push(phonemes);
                    continue;
                }
                let slices = split_into_syllables(&phonemes, syl_count);
                for slice in slices {
                    out.push(slice);
                }
            }
        }
    }
    out
}

/// Like `phonemes_for_draft` but also returns a parallel `Vec<bool>`
/// flagging the last syllable of each word. Lets the SVS pipeline
/// decide whether to insert a short `SP` (silence) between words for
/// crisper articulation.
pub fn phonemes_for_draft_with_word_boundaries(
    draft: &[crate::derive::LyricLine],
) -> (Vec<Vec<&'static str>>, Vec<bool>) {
    let mut tokens: Vec<LyricToken> = Vec::new();
    for line in draft {
        tokens.extend(tokenize_line(&line.text));
    }

    let mut phon: Vec<Vec<&'static str>> = Vec::new();
    let mut is_word_end: Vec<bool> = Vec::new();
    for token in tokens {
        let groups: Vec<Vec<&'static str>> = match token {
            LyricToken::PhonemeBlock(groups) => groups,
            LyricToken::Word { cleaned, syl_count, variant_idx } => {
                let phonemes = word_to_phonemes_variant(&cleaned, variant_idx);
                if syl_count == 1 {
                    vec![phonemes]
                } else {
                    split_into_syllables(&phonemes, syl_count)
                }
            }
        };
        let last = groups.len().saturating_sub(1);
        for (i, g) in groups.into_iter().enumerate() {
            phon.push(g);
            is_word_end.push(i == last);
        }
    }
    (phon, is_word_end)
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
