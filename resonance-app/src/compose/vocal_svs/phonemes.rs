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
//! Today the per-bank inventory is hardcoded (the historic enum
//! behaviour: every bank covers the full ARPAbet set except Lilia, which
//! lacks the voiced `v`). Epic #164's voicebank manifest scans the real
//! on-disk phoneme dict; todo #492 notes wiring that scanned set in here
//! as a follow-up so a freshly-dropped bank needs no code change. The
//! substitution policy here intentionally mirrors
//! `resonance_svs::voicebank`'s `nearest_substitutes` so the swap is a
//! drop-in.

use resonance_music_theory::{g2p, VocalVoicebank};

/// How a voicebank resolves one canonical ARPAbet symbol against its
/// phoneme inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhonemeFate {
    /// The bank's dict contains this symbol; it is sung as-is.
    Direct,
    /// The bank lacks this symbol; it is sung as the nearest available
    /// substitute instead (e.g. Lilia `v` → `f`).
    Substituted(&'static str),
    /// The bank lacks this symbol and has no acceptable substitute. The
    /// segment builder passes it through unchanged (matching the historic
    /// behaviour); the strip can badge it so the user knows it won't sing
    /// cleanly. No shipped bank currently hits this.
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
/// just wraps the enum); construct one per render / per strip-paint and
/// query it for each phone.
#[derive(Debug, Clone, Copy)]
pub struct VoicebankPhonemes {
    voicebank: VocalVoicebank,
}

impl VoicebankPhonemes {
    pub fn new(voicebank: VocalVoicebank) -> Self {
        Self { voicebank }
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
        for &candidate in nearest_substitutes(ph) {
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

    /// Is `ph` present in this bank's (hardcoded stand-in) inventory?
    /// Silence/control tokens are always present. Replace the
    /// [`missing_phonemes`] body with the scanned manifest set to make
    /// this data-driven (todo #492 follow-up).
    fn contains(&self, ph: &str) -> bool {
        !missing_phonemes(self.voicebank).contains(&ph)
    }
}

/// ARPAbet symbols *absent* from a bank's phoneme dict. Everything not
/// listed (including the `AP`/`SP`/`cl` control tokens) is treated as
/// present. This is the hardcoded stand-in for epic #164's scanned
/// inventory — see the module docs.
fn missing_phonemes(voicebank: VocalVoicebank) -> &'static [&'static str] {
    match voicebank {
        // TIGER (v106) and Meiji (v160) both ship the full English
        // ARPAbet set. Meiji namespaces it `en/…` on disk, but that's a
        // naming convention handled by `paths::voicebank_phoneme_name`,
        // not a missing symbol.
        VocalVoicebank::Tiger | VocalVoicebank::Meiji => &[],
        // Lilia's MM 2.8 set covers all of ARPAbet *except* the voiced
        // labiodental fricative `v`.
        VocalVoicebank::Lilia => &["v"],
    }
}

/// Nearest acceptable ARPAbet substitutes for a phone, most-similar
/// first. Consulted only for symbols a bank's dict is missing, so it
/// never alters a bank with the full inventory. Pairs voiced phones with
/// their voiceless counterpart (same place + manner) — the substitution
/// least likely to be noticed — with the reverse direction covering the
/// rarer voiceless-gap case. Mirrors `resonance_svs::voicebank`'s table
/// so the manifest swap stays behaviour-preserving.
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
