//! Pure coverage for the voicebank phonetic domain model.
//!
//! Everything here runs off an in-memory phoneme inventory — no voicebank
//! folder, no filesystem at all — which is the point of the layout /
//! phonetics split: the substitution table and the alphabet heuristic are
//! the parts with real musical consequence (a missing `v` is *sung* as `f`),
//! so they are asserted directly rather than through a scan.
//!
//! The `// was: <fn>(...)` comments quote the behaviour previously hardcoded
//! per `VocalVoicebank` arm in resonance-app's `vocal_svs/paths.rs`, kept so
//! any future change to the table is visibly a change in what banks sing.
//! `tests/voicebank_manifest.rs` covers the on-disk side and the wiring
//! between the two.

use std::collections::BTreeMap;

use resonance_svs::voicebank::{
    detect_phoneme_target, nearest_substitutes, CurveSupport, ExpressionCurve, PhonemeInventory,
    PhonemeTarget,
};

/// Full lowercase-ARPAbet inventory (silence markers, `cl`, every CMU
/// consonant + vowel) — the shape TIGER ships.
const ARPABET_FULL: &[&str] = &[
    "AP", "SP", "cl", "aa", "ae", "ah", "ao", "aw", "ay", "b", "ch", "d", "dh", "eh", "er", "ey",
    "f", "g", "hh", "ih", "iy", "jh", "k", "l", "m", "n", "ng", "ow", "oy", "p", "r", "s", "sh",
    "t", "th", "uh", "uw", "v", "w", "y", "z", "zh",
];

/// Meiji's namespaced dict: a bare universal bucket (silence + shared
/// consonants, notably no `en/hh`) plus the full English set under `en/`.
const MEIJI_DICT: &[&str] = &[
    "AP", "SP", "hh", "cl", "ban", "vf", "en/aa", "en/ae", "en/ah", "en/ao", "en/aw", "en/ay",
    "en/b", "en/ch", "en/d", "en/dh", "en/eh", "en/er", "en/ey", "en/f", "en/g", "en/ih", "en/iy",
    "en/jh", "en/k", "en/l", "en/m", "en/n", "en/ng", "en/ow", "en/oy", "en/p", "en/r", "en/s",
    "en/sh", "en/t", "en/th", "en/uh", "en/uw", "en/v", "en/w", "en/y", "en/z", "en/zh",
];

/// A representative spread of G2P-emitted ARPAbet symbols to exercise the
/// phoneme/language methods across silence markers, consonants and vowels.
const SAMPLE_PHONEMES: &[&str] = &["AP", "SP", "cl", "hh", "ah", "ae", "f", "v", "s", "t"];

/// TIGER-like: bare ARPAbet, nothing missing.
fn full_arpabet() -> PhonemeInventory {
    PhonemeInventory::new(ARPABET_FULL.iter().copied())
}

/// Lilia-like (LIEE Lilia, MM 2.8): every ARPAbet phone except the voiced
/// labiodental fricative `v`.
fn arpabet_without_v() -> PhonemeInventory {
    PhonemeInventory::new(ARPABET_FULL.iter().copied().filter(|ph| *ph != "v"))
}

/// Meiji-like: universal bucket bare, English namespaced under `en/`.
fn namespaced_english() -> PhonemeInventory {
    PhonemeInventory::new(MEIJI_DICT.iter().copied())
}

fn meiji_languages() -> BTreeMap<String, i64> {
    [("zh", 0), ("ja", 1), ("ko", 2), ("en", 3)]
        .into_iter()
        .map(|(name, id)| (name.to_string(), id))
        .collect()
}

// ---------------------------------------------------------------------------
// The substitution table itself.
// ---------------------------------------------------------------------------

#[test]
fn substitution_table_pairs_voiced_with_voiceless() {
    // Voiced -> voiceless counterpart first (same place + manner).
    assert_eq!(nearest_substitutes("v"), ["f", "b"]);
    assert_eq!(nearest_substitutes("dh"), ["th", "d"]);
    assert_eq!(nearest_substitutes("z"), ["s"]);
    assert_eq!(nearest_substitutes("zh"), ["sh"]);
    assert_eq!(nearest_substitutes("jh"), ["ch"]);
    // ... and the reverse direction for the rarer voiceless-gap case.
    assert_eq!(nearest_substitutes("f"), ["v"]);
    assert_eq!(nearest_substitutes("th"), ["dh", "t"]);
    assert_eq!(nearest_substitutes("s"), ["z"]);
    assert_eq!(nearest_substitutes("sh"), ["zh"]);
    assert_eq!(nearest_substitutes("ch"), ["jh"]);
}

#[test]
fn substitution_table_has_no_entry_for_vowels_or_silence() {
    // Nothing sensible to swap a vowel or a silence marker for: the phone
    // must pass through untouched rather than becoming another vowel.
    for ph in ["aa", "ae", "ah", "iy", "uw", "AP", "SP", "cl", "hh", "l", "m", "n"] {
        assert!(
            nearest_substitutes(ph).is_empty(),
            "{ph} must have no substitute"
        );
    }
}

#[test]
fn substitution_table_is_canonical_lowercase() {
    // Canonical phonemes are lowercase; an uppercase symbol is not a table
    // key, so it falls through to identity rather than silently matching.
    assert!(nearest_substitutes("V").is_empty());
    assert!(nearest_substitutes("F").is_empty());
}

// ---------------------------------------------------------------------------
// Substitution against an inventory.
// ---------------------------------------------------------------------------

#[test]
fn full_inventory_substitutes_nothing() {
    // was: substitute_phoneme(Tiger, ph) == ph (identity — full inventory).
    let inv = full_arpabet();
    for &ph in SAMPLE_PHONEMES {
        assert_eq!(inv.substitute_phoneme(ph), ph, "full sub {ph}");
    }
    for &ph in ARPABET_FULL {
        assert_eq!(inv.substitute_phoneme(ph), ph, "full sub {ph}");
    }
}

#[test]
fn missing_v_sings_as_f() {
    // was: substitute_phoneme(Lilia, "v") == "f" (the only documented sub).
    let inv = arpabet_without_v();
    assert_eq!(inv.substitute_phoneme("v"), "f");
    // Every other sampled phone is present, so substitution is identity.
    for &ph in SAMPLE_PHONEMES.iter().filter(|p| **p != "v") {
        assert_eq!(inv.substitute_phoneme(ph), ph, "lilia sub {ph}");
    }
}

#[test]
fn substitution_falls_through_to_the_second_candidate() {
    // A bank missing both `v` and its first choice `f` reaches the next
    // candidate, `b`.
    let inv = PhonemeInventory::new(
        ARPABET_FULL
            .iter()
            .copied()
            .filter(|ph| *ph != "v" && *ph != "f"),
    );
    assert_eq!(inv.substitute_phoneme("v"), "b");
}

#[test]
fn unsubstitutable_symbol_passes_through_unchanged() {
    // No candidate available (bank has neither v, f nor b) and symbols with
    // no table entry both pass through — the pipeline logs and maps them to
    // token 0 rather than singing an arbitrary phone.
    let inv = PhonemeInventory::new(
        ARPABET_FULL
            .iter()
            .copied()
            .filter(|ph| !matches!(*ph, "v" | "f" | "b")),
    );
    assert_eq!(inv.substitute_phoneme("v"), "v");

    let no_vowels = PhonemeInventory::new(["AP", "SP", "f", "s"]);
    assert_eq!(no_vowels.substitute_phoneme("ah"), "ah");
    assert_eq!(no_vowels.substitute_phoneme("zzz"), "zzz");
}

#[test]
fn namespaced_bank_substitutes_nothing() {
    // was: substitute_phoneme(Meiji, ph) == ph — the full English set is
    // present, just under `en/`, so the namespaced lookup must count as a
    // hit and suppress substitution.
    let inv = namespaced_english();
    for &ph in SAMPLE_PHONEMES {
        assert_eq!(inv.substitute_phoneme(ph), ph, "meiji sub {ph}");
    }
}

#[test]
fn namespaced_bank_substitutes_only_what_it_truly_lacks() {
    // Drop `en/v` from Meiji's dict: now `v` really is missing and falls
    // back to the (namespaced) `en/f` — reported as the bare `f`, since the
    // substitute is re-resolved through phoneme_name downstream.
    let inv = PhonemeInventory::new(MEIJI_DICT.iter().copied().filter(|ph| *ph != "en/v"));
    assert_eq!(inv.substitute_phoneme("v"), "f");
    assert_eq!(inv.phoneme_name("f"), "en/f");
}

// ---------------------------------------------------------------------------
// Dict-key resolution and language ids.
// ---------------------------------------------------------------------------

#[test]
fn bare_inventory_resolves_bare_keys() {
    // was: voicebank_phoneme_name(Tiger|Lilia, ph) == ph (bare ARPAbet).
    let inv = full_arpabet();
    for &ph in SAMPLE_PHONEMES {
        assert_eq!(inv.phoneme_name(ph), ph, "bare name {ph}");
        assert_eq!(inv.dict_key_for(ph).as_deref(), Some(ph));
    }
    assert_eq!(inv.dict_key_for("zzz"), None);
    // Unknown symbols pass through phoneme_name unchanged.
    assert_eq!(inv.phoneme_name("zzz"), "zzz");
}

#[test]
fn namespaced_inventory_prefixes_english_and_keeps_the_universal_bucket_bare() {
    // was: voicebank_phoneme_name(Meiji, ph) — bucket bare, rest `en/`.
    let inv = namespaced_english();
    for &uni in &["AP", "SP", "cl", "hh"] {
        assert_eq!(inv.phoneme_name(uni), uni, "universal {uni}");
    }
    for &(ph, want) in &[("ah", "en/ah"), ("ae", "en/ae"), ("f", "en/f"), ("v", "en/v")] {
        assert_eq!(inv.phoneme_name(ph), want, "namespaced name {ph}");
    }
}

#[test]
fn language_ids_follow_the_namespace() {
    // was: voicebank_language_id(Meiji, ph) — 0 for the bucket, 3 for English.
    let inv = namespaced_english();
    let langs = meiji_languages();
    for &uni in &["AP", "SP", "cl", "hh"] {
        assert_eq!(inv.language_id(uni, &langs), 0, "universal lang {uni}");
    }
    for &ph in &["ah", "ae", "f", "v"] {
        assert_eq!(inv.language_id(ph, &langs), 3, "english lang {ph}");
    }
    // A namespace absent from languages.json degrades to the default id.
    let inv_unknown_ns = PhonemeInventory::new(["AP", "en/ah"]);
    assert_eq!(inv_unknown_ns.language_id("ah", &BTreeMap::new()), 0);
}

#[test]
fn bare_inventory_reports_the_default_language() {
    // Single-language banks store bare keys, so every token is language 0;
    // the manifest is what turns this into `None` when the model takes no
    // languages input.
    let inv = full_arpabet();
    let langs = meiji_languages();
    for &ph in SAMPLE_PHONEMES {
        assert_eq!(inv.language_id(ph, &langs), 0, "bare lang {ph}");
    }
}

// ---------------------------------------------------------------------------
// Alphabet detection.
// ---------------------------------------------------------------------------

#[test]
fn arpabet_inventories_are_detected_as_arpabet() {
    // was: every VocalVoicebank uses bare/`en/`-prefixed ARPAbet, none x-sampa.
    assert_eq!(full_arpabet().target(), PhonemeTarget::Arpabet);
    assert_eq!(arpabet_without_v().target(), PhonemeTarget::Arpabet);
    assert_eq!(namespaced_english().target(), PhonemeTarget::Arpabet);
    assert_eq!(detect_phoneme_target(ARPABET_FULL), PhonemeTarget::Arpabet);
    assert_eq!(detect_phoneme_target(MEIJI_DICT), PhonemeTarget::Arpabet);
    assert_eq!(
        detect_phoneme_target::<[&str; 0], &str>([]),
        PhonemeTarget::Arpabet,
        "an empty inventory is not evidence of X-SAMPA"
    );
}

#[test]
fn xsampa_glyphs_flip_the_verdict() {
    // One X-SAMPA-only glyph anywhere in the inventory is enough.
    for glyph in ["@", "{", "}", "r\\", "~", "=", "&", "|"] {
        let inv = PhonemeInventory::new(["AP", "SP", "a", glyph]);
        assert_eq!(
            inv.target(),
            PhonemeTarget::XSampa,
            "glyph {glyph} should read as X-SAMPA"
        );
    }
    assert_eq!(
        detect_phoneme_target(["AP", "SP", "@", "{", "r\\", "O"]),
        PhonemeTarget::XSampa
    );
}

// ---------------------------------------------------------------------------
// Expression-curve capabilities.
// ---------------------------------------------------------------------------

#[test]
fn pitch_bend_is_always_available() {
    // Pitch bend is a pre-synthesis f0 edit, so no acoustic input gates it.
    assert!(CurveSupport::default().supports(ExpressionCurve::PitchBend));
}

#[test]
fn curve_support_follows_the_acoustic_embeds() {
    // was: curve_supported(Tiger, …) — dynamics/pitch yes, tension/breath no.
    let tiger = CurveSupport {
        energy: true,
        ..CurveSupport::default()
    };
    assert!(tiger.supports(ExpressionCurve::Dynamics));
    assert!(tiger.supports(ExpressionCurve::PitchBend));
    assert!(!tiger.supports(ExpressionCurve::Tension));
    assert!(!tiger.supports(ExpressionCurve::Breathiness));

    // was: curve_supported(Lilia|Meiji, …) — all four supported.
    let full = CurveSupport {
        energy: true,
        tension: true,
        breathiness: true,
    };
    for c in [
        ExpressionCurve::Dynamics,
        ExpressionCurve::Tension,
        ExpressionCurve::Breathiness,
        ExpressionCurve::PitchBend,
    ] {
        assert!(full.supports(c), "full curve {c:?}");
    }

    // A bank with no embeds at all still offers pitch bend and nothing else.
    let none = CurveSupport::default();
    assert!(!none.supports(ExpressionCurve::Dynamics));
    assert!(!none.supports(ExpressionCurve::Tension));
    assert!(!none.supports(ExpressionCurve::Breathiness));
}
