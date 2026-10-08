//! Tests for [`VoicebankPhonemes`] — the single source of truth shared
//! by the SVS segment builder and the vocal-roll phoneme strip (design
//! #173, todo #492).
//!
//! These pin two things: (1) every shipped bank sings every ARPAbet phone
//! the G2P emits as itself — Lilia included, since her dictionary's
//! replacements say how she spells English `v` (as her own `w`), where she
//! used to substitute `f` — and (2) `effective`, what the strip displays,
//! agrees with what the render sings for every phone, so the strip and the
//! render can never disagree.

use resonance_app::compose::vocal_svs::{BankProfile, PhonemeFate, VoicebankPhonemes};
use resonance_music_theory::g2p::ARPABET_PHONEMES;

/// A shipped bank as the built-in table describes it (nothing on disk).
fn bank(id: &str) -> BankProfile {
    BankProfile::not_installed(id)
}

#[test]
fn every_shipped_bank_sings_every_phone_directly() {
    for vb in ["tiger", "lilia", "meiji"] {
        let profile = bank(vb);
        let bank = VoicebankPhonemes::new(&profile);
        for &ph in ARPABET_PHONEMES {
            assert_eq!(bank.resolve(ph), PhonemeFate::Direct, "{vb:?} altered {ph}");
            assert_eq!(bank.effective(ph), ph, "{vb:?} effective({ph})");
        }
        // Silence/control tokens are passed through untouched on every
        // bank — the segment builder relies on this for its AP/SP pads.
        for marker in ["AP", "SP", "cl"] {
            assert_eq!(bank.effective(marker), marker, "{vb:?} mangled {marker}");
        }
    }
}

#[test]
fn lilia_spells_english_with_her_own_symbols() {
    // MM 2.8's `dsdict-en.yaml` replacements: what the model is fed for
    // each English phone it does not hold under the ARPAbet name.
    let lilia = bank("lilia");
    for (ph, key) in [
        ("t", "t0"),
        ("d", "d0"),
        ("r", "r0"),
        ("l", "ll"),
        ("v", "w"),
        ("w", "L"),
        ("y", "j"),
        ("ah", "ah"),
        ("s", "s"),
    ] {
        assert_eq!(lilia.phoneme_name(ph), key, "{ph}");
    }
    // Spelling is not substitution: the phone is sung, as itself.
    assert!(VoicebankPhonemes::new(&lilia).is_supported("v"));
    assert_eq!(bank("tiger").phoneme_name("w"), "w");
    assert_eq!(bank("meiji").phoneme_name("w"), "en/w");
}

#[test]
fn valid_set_is_full_arpabet_on_every_shipped_bank() {
    for vb in ["tiger", "lilia", "meiji"] {
        let profile = bank(vb);
        assert_eq!(VoicebankPhonemes::new(&profile).valid_set(), ARPABET_PHONEMES, "{vb}");
    }
}

#[test]
fn valid_set_preserves_canonical_order() {
    // The set must come back in ARPABET_PHONEMES order so the UI can show
    // a stable inventory; filtering must not reshuffle it.
    let profile = bank("lilia");
    let lilia = VoicebankPhonemes::new(&profile).valid_set();
    let mut sorted = lilia.clone();
    sorted.sort_by_key(|ph| ARPABET_PHONEMES.iter().position(|p| p == ph));
    assert_eq!(lilia, sorted);
}

#[test]
fn phoneme_fate_effective_picks_substitute() {
    assert_eq!(PhonemeFate::Direct.effective("ah"), "ah");
    assert_eq!(PhonemeFate::Substituted("f").effective("v"), "f");
    // Unsupported sings the original unchanged (the builder passes it
    // through; the strip flags it separately).
    assert_eq!(PhonemeFate::Unsupported.effective("xx"), "xx");
}
