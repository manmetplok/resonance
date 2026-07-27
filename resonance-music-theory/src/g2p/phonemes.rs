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
const CMUDICT_TEXT: &str = include_str!("../../data/cmudict.dict");

pub(crate) fn dict() -> &'static Cmudict {
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
pub(crate) fn word_to_phonemes_variant(
    word: &str,
    variant_idx: usize,
) -> Vec<(&'static str, SyllableStress)> {
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
