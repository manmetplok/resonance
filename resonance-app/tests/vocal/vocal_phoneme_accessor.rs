//! Tests for [`VoicebankPhonemes`] — the single source of truth shared
//! by the SVS segment builder and the vocal-roll phoneme strip (design
//! #173, todo #492).
//!
//! These pin two things: (1) the accessor reproduces the historic
//! per-voicebank substitution behaviour the segment builder relied on
//! (every bank covers the full ARPAbet set except Lilia, which lacks the
//! voiced `v` and substitutes `f`), and (2) `effective` — what the strip
//! displays — agrees with that substitution for every phone the G2P can
//! emit, so the strip and the render can never disagree.

use resonance_app::compose::vocal_svs::{BankProfile, PhonemeFate, VoicebankPhonemes};
use resonance_music_theory::g2p::ARPABET_PHONEMES;

/// A shipped bank as the built-in table describes it (nothing on disk).
fn bank(id: &str) -> BankProfile {
    BankProfile::not_installed(id)
}

/// The historic hardcoded `substitute_phoneme` mapping, reproduced here
/// so the accessor is checked against an independent statement of the
/// old behaviour rather than its own internals.
fn legacy_substitute<'a>(vb: &str, ph: &'a str) -> &'a str {
    match vb {
        "lilia" if ph == "v" => "f",
        _ => ph,
    }
}

#[test]
fn effective_matches_legacy_substitution_for_every_phone() {
    for vb in ["tiger", "lilia", "meiji"] {
        let profile = bank(vb);
        let bank = VoicebankPhonemes::new(&profile);
        for &ph in ARPABET_PHONEMES {
            assert_eq!(
                bank.effective(ph),
                legacy_substitute(vb, ph),
                "{vb:?} effective({ph}) diverged from the historic substitution"
            );
        }
        // Silence/control tokens are passed through untouched on every
        // bank — the segment builder relies on this for its AP/SP pads.
        for marker in ["AP", "SP", "cl"] {
            assert_eq!(bank.effective(marker), marker, "{vb:?} mangled {marker}");
        }
    }
}

#[test]
fn lilia_resolves_v_as_a_substitution_others_direct() {
    let profile = bank("lilia");
    let lilia = VoicebankPhonemes::new(&profile);
    assert_eq!(lilia.resolve("v"), PhonemeFate::Substituted("f"));
    assert_eq!(lilia.resolve("f"), PhonemeFate::Direct);
    assert_eq!(lilia.resolve("ah"), PhonemeFate::Direct);
    // Substituted phones still count as supported (they sing, just as a
    // near neighbour) — the strip badges only the Unsupported case.
    assert!(lilia.is_supported("v"));
}

#[test]
fn full_inventory_banks_substitute_nothing() {
    for vb in ["tiger", "meiji"] {
        let profile = bank(vb);
        let bank = VoicebankPhonemes::new(&profile);
        for &ph in ARPABET_PHONEMES {
            assert_eq!(
                bank.resolve(ph),
                PhonemeFate::Direct,
                "{vb:?} unexpectedly altered {ph}"
            );
        }
    }
}

#[test]
fn valid_set_is_full_arpabet_except_lilias_v() {
    let tiger_profile = bank("tiger");
    let tiger = VoicebankPhonemes::new(&tiger_profile);
    assert_eq!(tiger.valid_set(), ARPABET_PHONEMES);

    let lilia_profile = bank("lilia");
    let lilia = VoicebankPhonemes::new(&lilia_profile);
    let expected: Vec<&str> = ARPABET_PHONEMES
        .iter()
        .copied()
        .filter(|&ph| ph != "v")
        .collect();
    assert_eq!(lilia.valid_set(), expected);
    assert!(!lilia.valid_set().contains(&"v"));
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
