//! The phonetic domain model of a voicebank: which alphabet its dictionary
//! is written in, how a G2P symbol maps onto a dict key, what to sing when
//! the bank is missing a phone, and which expression curves its model can
//! take.
//!
//! Nothing here touches the filesystem. Everything is a pure function of an
//! in-memory phoneme inventory (plus, for language ids, the bank's
//! `languages.json` map), so the substitution table and the alphabet
//! heuristic can be exercised without a voicebank folder on disk. The
//! on-disk side — locating and reading those inventories — lives in
//! [`super::layout`].

use std::collections::{BTreeMap, HashSet};

/// The phoneme alphabet a voicebank's dictionary is written in. The
/// Resonance G2P emits English ARPAbet (lowercase, stress-stripped); a
/// bank whose inventory is X-SAMPA needs a different symbol set, so the
/// pipeline can warn rather than feed it mismatched tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhonemeTarget {
    /// Lowercase ARPAbet (`ah`, `ae`, `f`, ...) — what every shipped bank
    /// and the G2P use.
    Arpabet,
    /// X-SAMPA (`@`, `{`, `r\`, ...). Detected but not yet G2P-supported.
    XSampa,
}

/// One of the four editable vocal expression curves. Mirrors the
/// `CurveKind` the vocal-roll Expression dock and SVS segment builder use;
/// kept here so [`CurveSupport::supports`] can answer the capability
/// question from the bank's acoustic-config flags rather than a per-bank
/// enum match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExpressionCurve {
    /// Dynamics / energy (loudness envelope) — needs the `energy` embed.
    Dynamics,
    /// Vocal tension — needs the `tension` embed.
    Tension,
    /// Breathiness — needs the `breathiness` embed.
    Breathiness,
    /// Pitch bend — applied as a pre-synthesis f0 edit, so always
    /// available regardless of the acoustic model's inputs.
    PitchBend,
}

/// Which curve inputs a bank's acoustic model actually accepts, lifted out
/// of the acoustic config so the capability answer is a pure function of
/// three booleans.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CurveSupport {
    /// Model takes an `energy` input (drives Dynamics).
    pub energy: bool,
    /// Model takes a `tension` input.
    pub tension: bool,
    /// Model takes a `breathiness` input.
    pub breathiness: bool,
}

impl CurveSupport {
    /// Whether the given expression curve is usable on this bank.
    ///
    /// Pitch bend is a pre-synthesis f0 edit, so it is always available;
    /// the other three each require their matching acoustic embed. TIGER's
    /// model exposes no tension or breathiness input, so those report
    /// `false` there while Lilia and Meiji accept all three.
    pub fn supports(&self, curve: ExpressionCurve) -> bool {
        match curve {
            ExpressionCurve::PitchBend => true,
            ExpressionCurve::Dynamics => self.energy,
            ExpressionCurve::Tension => self.tension,
            ExpressionCurve::Breathiness => self.breathiness,
        }
    }
}

/// The language the Resonance G2P transcribes into. All transcription is
/// English ARPAbet today; banks that namespace phonemes by language (Gahata
/// Meiji) prefix the English set with `en/`, so this is the prefix
/// [`PhonemeInventory::phoneme_name`] reaches for when a bare symbol is
/// absent.
pub const G2P_LANGUAGE: &str = "en";

/// The set of phoneme names one voicebank's dictionary declares, plus the
/// alphabet detected from them.
///
/// Built once per bank (at scan time, or straight from a literal list in a
/// test) and then queried per token on the render path, so membership is a
/// `HashSet` lookup rather than a scan.
#[derive(Debug, Clone)]
pub struct PhonemeInventory {
    names: HashSet<String>,
    target: PhonemeTarget,
}

impl PhonemeInventory {
    /// Build an inventory from a bank's phoneme names, in any order.
    pub fn new<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let names: HashSet<String> = names.into_iter().map(Into::into).collect();
        let target = detect_phoneme_target(names.iter());
        Self { names, target }
    }

    /// Alphabet detected from the inventory.
    pub fn target(&self) -> PhonemeTarget {
        self.target
    }

    /// Number of declared phonemes.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// True when the bank declared no phonemes at all.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Resolve a bare G2P ARPAbet symbol to the dict key this bank actually
    /// stores it under, or `None` if neither the bare form nor the
    /// language-namespaced (`en/…`) form is in the inventory.
    ///
    /// Single-language banks (TIGER, Lilia) store bare ARPAbet, so the bare
    /// lookup hits. Banks that namespace by language (Meiji) keep a small
    /// universal bucket bare (`AP`, `SP`, `hh`, `cl`, …) and prefix the
    /// rest, so the `en/` fallback hits for those.
    pub fn dict_key_for(&self, ph: &str) -> Option<String> {
        if self.names.contains(ph) {
            return Some(ph.to_string());
        }
        let namespaced = format!("{G2P_LANGUAGE}/{ph}");
        if self.names.contains(&namespaced) {
            return Some(namespaced);
        }
        None
    }

    /// The dict key to feed the acoustic model for a G2P ARPAbet symbol —
    /// bare for single-language banks, `en/`-prefixed for the namespaced
    /// portion of a multi-language bank. Falls back to the bare symbol when
    /// the bank stores it in neither form (matching the old code, which
    /// passed unknown symbols through unchanged).
    pub fn phoneme_name(&self, ph: &str) -> String {
        self.dict_key_for(ph).unwrap_or_else(|| ph.to_string())
    }

    /// Per-token id for the acoustic model's `languages` input.
    ///
    /// Namespaced symbols (`en/ah`) report their language's id from
    /// `languages.json`; the bare universal bucket (silence markers and
    /// shared consonants) reports `0`, the default/silence language —
    /// reproducing Meiji's `0` for `AP`/`hh`/… and `3` for English. Banks
    /// whose model takes no `languages` input never ask (see
    /// [`super::VoicebankManifest::language_id`]).
    pub fn language_id(&self, ph: &str, languages: &BTreeMap<String, i64>) -> i64 {
        let key = self.phoneme_name(ph);
        match key.split_once('/') {
            Some((lang, _)) => languages.get(lang).copied().unwrap_or(0),
            None => 0,
        }
    }

    /// Replace a G2P ARPAbet symbol the bank's dict lacks with its nearest
    /// available substitute; symbols the bank knows pass through unchanged.
    ///
    /// Only fires when the symbol resolves to neither a bare nor a
    /// namespaced dict key — e.g. Lilia ships every ARPAbet phone except
    /// the voiced `v`, so `v` (absent) maps to `f` (its voiceless
    /// counterpart, present) while everything else is left alone. Banks
    /// with the full inventory (TIGER, Meiji) substitute nothing.
    pub fn substitute_phoneme(&self, ph: &str) -> String {
        if self.dict_key_for(ph).is_some() {
            return ph.to_string();
        }
        for candidate in nearest_substitutes(ph) {
            if self.dict_key_for(candidate).is_some() {
                return candidate.to_string();
            }
        }
        ph.to_string()
    }
}

/// Nearest acceptable ARPAbet substitutes for a phone, most-similar first.
/// Consulted only for symbols a bank's dict is missing, so it never alters
/// a bank with the full inventory. Pairs voiced phones with their voiceless
/// counterpart (same place + manner), the substitution least likely to be
/// noticed; the reverse direction covers the rarer voiceless-gap case.
pub fn nearest_substitutes(ph: &str) -> &'static [&'static str] {
    match ph {
        "v" => &["f", "b"],
        "f" => &["v"],
        "dh" => &["th", "d"],
        "th" => &["dh", "t"],
        "z" => &["s"],
        "s" => &["z"],
        "zh" => &["sh"],
        "sh" => &["zh"],
        "jh" => &["ch"],
        "ch" => &["jh"],
        _ => &[],
    }
}

/// Detect a bank's phoneme alphabet from its inventory. ARPAbet here is
/// lowercase ASCII letters with optional `lang/` prefixes; X-SAMPA mixes in
/// glyphs ARPAbet never uses (`@ { } \ ~ = & |` and bare uppercase
/// vowels like `O`/`I`/`E`). Any such glyph flips the verdict to X-SAMPA.
pub fn detect_phoneme_target<I, S>(phonemes: I) -> PhonemeTarget
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let is_xsampa_glyph = |c: char| matches!(c, '@' | '{' | '}' | '\\' | '~' | '=' | '&' | '|');
    for ph in phonemes {
        if ph.as_ref().chars().any(is_xsampa_glyph) {
            return PhonemeTarget::XSampa;
        }
    }
    PhonemeTarget::Arpabet
}
