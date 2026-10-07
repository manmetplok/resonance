//! Single source of truth for *which ARPAbet phonemes the active
//! voicebank can actually sing, and what substitution applies* (design
//! #173 key decision).
//!
//! Two consumers must never disagree on this: the pronunciation
//! validation gate ([`super::validate_for_voicebank`], whose substituted
//! output the segment builder feeds the model) decides what tokens the
//! model is fed, and the vocal-roll phoneme strip shows the user what the
//! model will sing. If they each carried
//! their own table, the strip could display `v` while the model sang
//! `f`. Both now go through [`VoicebankPhonemes`].
//!
//! The inventory is the bank's: an installed bank answers from its
//! scanned phoneme dictionary, a known bank that is not installed from the
//! built-in record of its shipped one ([`BankProfile::has_phoneme`]). A
//! phone the bank lacks is mapped onto what it has
//! (vocal-mcp-control-and-quality.md §4.4): the bank's own substitution
//! hints first (Lilia `v` → `f`, `b`), then the voiced/voiceless partner,
//! then the nearest phone of the same articulatory class
//! ([`nearest_by_class`]). Only a phone none of those reach is
//! unsupported.

use resonance_music_theory::g2p;

use super::registry::BankProfile;

/// How a voicebank resolves one canonical ARPAbet symbol against its
/// phoneme inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhonemeFate {
    /// The bank's dict contains this symbol; it is sung as-is.
    Direct,
    /// The bank lacks this symbol; it is sung as the nearest available
    /// substitute instead (e.g. Lilia `v` → `f`).
    Substituted(&'static str),
    /// The bank lacks this symbol and nothing it has is close enough. The
    /// pronunciation gate refuses the render, naming the word; the strip
    /// shows the symbol as written. No shipped bank hits this.
    Unsupported,
}

impl PhonemeFate {
    /// The symbol actually sung for the queried phone. `Direct` and
    /// `Unsupported` sing the original; `Substituted` sings the
    /// substitute.
    pub fn effective<'a>(&self, original: &'a str) -> &'a str {
        match self {
            // The `&'static str` substitute coerces to the shorter `'a`.
            PhonemeFate::Substituted(sub) => sub,
            PhonemeFate::Direct | PhonemeFate::Unsupported => original,
        }
    }
}

/// The active voicebank's phoneme capabilities. Cheap to construct (it
/// borrows the bank's profile); construct one per render / per strip-paint
/// and query it for each phone.
#[derive(Debug, Clone, Copy)]
pub struct VoicebankPhonemes<'a> {
    bank: &'a BankProfile,
}

impl<'a> VoicebankPhonemes<'a> {
    pub fn new(bank: &'a BankProfile) -> Self {
        Self { bank }
    }

    /// Resolve one canonical ARPAbet symbol (as the G2P emits it) to its
    /// fate in this bank. Silence/control tokens (`AP`, `SP`, `cl`) and
    /// any symbol the bank's inventory already covers are [`Direct`].
    ///
    /// [`Direct`]: PhonemeFate::Direct
    pub fn resolve(&self, ph: &str) -> PhonemeFate {
        if self.contains(ph) {
            return PhonemeFate::Direct;
        }
        let candidates = self
            .bank
            .substitution_hints(ph)
            .iter()
            .chain(nearest_substitutes(ph))
            .chain(nearest_by_class(ph));
        for &candidate in candidates {
            if self.contains(candidate) {
                return PhonemeFate::Substituted(candidate);
            }
        }
        PhonemeFate::Unsupported
    }

    /// The symbol this bank actually sings for `ph` — `ph` itself when
    /// it's directly singable (or unsupported), the substitute when one
    /// applies. This is what the segment builder feeds the model and what
    /// the strip displays, so they agree by construction.
    pub fn effective(&self, ph: &'static str) -> &'static str {
        self.resolve(ph).effective(ph)
    }

    /// Whether `ph` will sing without being dropped or mangled — `true`
    /// for [`Direct`] and [`Substituted`], `false` for [`Unsupported`].
    ///
    /// [`Direct`]: PhonemeFate::Direct
    /// [`Substituted`]: PhonemeFate::Substituted
    /// [`Unsupported`]: PhonemeFate::Unsupported
    pub fn is_supported(&self, ph: &str) -> bool {
        !matches!(self.resolve(ph), PhonemeFate::Unsupported)
    }

    /// Every ARPAbet phone this bank sings directly (no substitution), in
    /// canonical [`g2p::ARPABET_PHONEMES`] order. This is the bank's
    /// effective lexical inventory — the strip uses it to validate
    /// power-user phoneme overrides, the segment builder relies on it
    /// implicitly through [`Self::effective`].
    pub fn valid_set(&self) -> Vec<&'static str> {
        g2p::ARPABET_PHONEMES
            .iter()
            .copied()
            .filter(|ph| self.contains(ph))
            .collect()
    }

    /// Is `ph` in this bank's inventory? Control tokens always are.
    fn contains(&self, ph: &str) -> bool {
        self.bank.has_phoneme(ph)
    }
}

// ---------------------------------------------------------------------------
// Articulation timing
// ---------------------------------------------------------------------------
//
// How long each phone needs to be *heard* as itself. Two consumers share
// this table and must not disagree: the segment builder
// (`segment::duration`) hands out the real `ph_dur` slices, and the
// intelligibility report (`validate::articulation_report`) tells a client
// over the wire whether a note is long enough before they bounce audio.
//
// A uniform per-consonant duration is the wrong model. A `t` burst is
// perceptually complete in ~30 ms while an `s` needs 3-4× that before the
// frication is identifiable, and a schwa vowel squeezed under ~50 ms stops
// carrying a formant target at all. With one number for every consonant,
// a phoneme-dense syllable divided its slice evenly and every segment came
// out below its own audibility floor — the whole syllable turned to mush
// rather than one phone being clipped.

/// Manner-of-articulation class of an ARPAbet symbol. Duration demands
/// differ by manner far more than by place, so this is the axis the
/// timing table is keyed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArticulationClass {
    /// Any vowel or diphthong — the syllable nucleus, which absorbs
    /// whatever time the consonants leave.
    Vowel,
    /// Oral stop (`b p t d k g`) plus `hh`: a brief burst / aspiration.
    Stop,
    /// Affricate (`ch jh`): stop closure followed by frication.
    Affricate,
    /// Fricative (`f v th dh s z sh zh`): needs sustained noise before
    /// its spectrum is identifiable — the longest consonant class.
    Fricative,
    /// Nasal (`m n ng`): voiced murmur, moderately long.
    Nasal,
    /// Liquid or glide (`l r w y`): a formant transition more than a
    /// segment of its own.
    Approximant,
}

/// Classify one canonical ARPAbet symbol. Silence / control tokens
/// (`AP`, `SP`, `cl`) are not lexical phones and never reach here; they
/// classify as [`ArticulationClass::Stop`] (the shortest class) if they
/// somehow do.
pub fn articulation_class(ph: &str) -> ArticulationClass {
    match ph {
        "b" | "p" | "t" | "d" | "k" | "g" | "hh" => ArticulationClass::Stop,
        "ch" | "jh" => ArticulationClass::Affricate,
        "f" | "v" | "th" | "dh" | "s" | "z" | "sh" | "zh" => ArticulationClass::Fricative,
        "m" | "n" | "ng" => ArticulationClass::Nasal,
        "l" | "r" | "w" | "y" => ArticulationClass::Approximant,
        _ if g2p::is_consonant(ph) => ArticulationClass::Approximant,
        _ => ArticulationClass::Vowel,
    }
}

impl ArticulationClass {
    /// The `(relaxed, deliberate)` target duration band in seconds,
    /// interpolated by the lane's `consonant_emphasis`. Vowels have no
    /// target — they take the remainder of the note — so their band is
    /// only a lower bound used when a syllable is all-vowel.
    ///
    /// The bands are centred on the old uniform 35-85 ms so the existing
    /// characterisation of `consonant_emphasis` (garbage below ~0.15,
    /// mis-heard endings above ~0.60) still holds; what changed is the
    /// *distribution* across classes, not the average.
    fn target_band(self) -> (f64, f64) {
        match self {
            ArticulationClass::Vowel => (0.090, 0.090),
            ArticulationClass::Stop => (0.035, 0.060),
            ArticulationClass::Affricate => (0.055, 0.090),
            ArticulationClass::Fricative => (0.055, 0.095),
            ArticulationClass::Nasal => (0.045, 0.075),
            ArticulationClass::Approximant => (0.040, 0.070),
        }
    }

    /// Absolute floor in seconds: below this the phone is not heard as
    /// itself, so shortening past it destroys information rather than
    /// merely rushing it. Used both to protect consonants when a note is
    /// tight and to compute [`min_articulation_sec`].
    pub fn floor_sec(self) -> f64 {
        match self {
            ArticulationClass::Vowel => 0.050,
            ArticulationClass::Stop => 0.030,
            ArticulationClass::Affricate => 0.045,
            ArticulationClass::Fricative => 0.040,
            ArticulationClass::Nasal => 0.035,
            ArticulationClass::Approximant => 0.035,
        }
    }
}

/// Target duration in seconds for one phone at `consonant_emphasis`
/// `emphasis` (0..1). Vowels report their floor-ish nominal; the segment
/// builder overrides it with whatever the consonants leave over.
pub fn target_duration_sec(ph: &str, emphasis: f32) -> f64 {
    let (lo, hi) = articulation_class(ph).target_band();
    lo + (hi - lo) * emphasis.clamp(0.0, 1.0) as f64
}

/// Floor duration in seconds for one phone — see
/// [`ArticulationClass::floor_sec`].
pub fn floor_duration_sec(ph: &str) -> f64 {
    articulation_class(ph).floor_sec()
}

/// The shortest note, in seconds, that can articulate `phonemes` without
/// pushing any of them below its audibility floor. This is the number the
/// `song.vocal` intelligibility report compares a note's real duration
/// against: a note shorter than this **will** sound like a smear no matter
/// what the model does, and the fix is a longer note or fewer phonemes on
/// it (i.e. more syllable breaks), not a parameter tweak.
///
/// An empty phoneme list (a note the lyric draft never reached) needs no
/// time and reports `0.0`.
pub fn min_articulation_sec(phonemes: &[&str]) -> f64 {
    phonemes.iter().map(|p| floor_duration_sec(p)).sum()
}

/// Nearest acceptable ARPAbet substitutes for a phone, most-similar
/// first. Consulted only for symbols a bank's dict is missing, so it
/// never alters a bank with the full inventory. Pairs voiced phones with
/// their voiceless counterpart (same place + manner) — the substitution
/// least likely to be noticed — with the reverse direction covering the
/// rarer voiceless-gap case. Mirrors `resonance_svs::voicebank`'s table.
fn nearest_substitutes(ph: &str) -> &'static [&'static str] {
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

/// The phones of the same articulatory class nearest to `ph`, most similar
/// first: the last resort before a phone counts as unsupported. Vowels map
/// to vowels by height and backness, consonants to consonants of the same
/// manner (then place), so a substitution never changes a syllable's vowel
/// count. Voicing partners come earlier, from [`nearest_substitutes`].
pub fn nearest_by_class(ph: &str) -> &'static [&'static str] {
    match ph {
        // Vowels.
        "aa" => &["ao", "ah", "ae"],
        "ae" => &["eh", "aa", "ah"],
        "ah" => &["ax", "aa", "uh"],
        "ax" => &["ah", "ih", "eh"],
        "ao" => &["aa", "ow", "ah"],
        "aw" => &["ao", "aa", "ow"],
        "ay" => &["aa", "ey", "ah"],
        "eh" => &["ae", "ih", "ey"],
        "er" => &["ah", "ax"],
        "ey" => &["eh", "iy", "ih"],
        "ih" => &["iy", "eh", "ax"],
        "iy" => &["ih", "ey"],
        "ow" => &["ao", "uw", "ah"],
        "oy" => &["ow", "ao"],
        "uh" => &["uw", "ah", "ax"],
        "uw" => &["uh", "ow"],
        // Stops.
        "b" => &["p", "d"],
        "p" => &["b", "t"],
        "d" => &["t", "b"],
        "t" => &["d", "k"],
        "g" => &["k", "d"],
        "k" => &["g", "t"],
        // Affricates.
        "ch" => &["sh", "t"],
        "jh" => &["zh", "d"],
        // Fricatives.
        "f" => &["th", "p"],
        "v" => &["dh", "w"],
        "th" => &["f", "s"],
        "dh" => &["z", "v"],
        "s" => &["sh", "th"],
        "z" => &["zh", "dh"],
        "sh" => &["s", "ch"],
        "zh" => &["z", "jh"],
        "hh" => &["f", "th"],
        // Nasals.
        "m" => &["n"],
        "n" => &["m", "ng"],
        "ng" => &["n", "m"],
        // Approximants.
        "l" => &["r", "w"],
        "r" => &["l", "w"],
        "w" => &["l", "r"],
        "y" => &["l", "r"],
        _ => &[],
    }
}
