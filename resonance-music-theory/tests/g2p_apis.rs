//! Tests for the public G2P pronunciation APIs the app's phoneme strip,
//! add-phoneme palette and dictionary need: CMU variant enumeration, the
//! ARPAbet inventory list, public phoneme validation, and the
//! dictionary / per-syllable override resolution hooks (with provenance).

use std::collections::HashMap;

use resonance_music_theory::g2p::{
    assign_syllables_to_notes, assign_syllables_to_notes_with, canonical_phoneme, cmu_variant_count,
    cmu_variants, is_consonant, phonemes_for_draft, resolve_draft, resolve_draft_with_dict,
    PhonemeDictionary, PhonemeProvenance, SyllableOverrides, SyllableStress, ARPABET_SYMBOLS,
};
use resonance_music_theory::LyricLine;

fn line(text: &str) -> LyricLine {
    LyricLine {
        n: 1,
        rhyme: 'A',
        syllables: 0,
        text: text.into(),
        locked: false,
    }
}

// ---------------------------------------------------------------------
// (1) CMU variant enumeration
// ---------------------------------------------------------------------

#[test]
fn cmu_variants_count_matches_cmu_variant_count() {
    for word in ["read", "live", "the", "xyzzy", "house"] {
        assert_eq!(
            cmu_variants(word).len(),
            cmu_variant_count(word),
            "variant list length disagrees with cmu_variant_count for {word:?}"
        );
    }
}

#[test]
fn cmu_variants_are_one_indexed_in_order() {
    let variants = cmu_variants("read");
    for (i, v) in variants.iter().enumerate() {
        assert_eq!(v.index, i + 1, "variant index should be 1-based and ordered");
    }
}

#[test]
fn cmu_variants_phonemes_match_lyric_variant_hint() {
    // Each enumerated variant must sing exactly what the `word(N)` lyric
    // hint produces — the picker and the lyric path can't diverge.
    let variants = cmu_variants("read");
    assert!(variants.len() >= 2, "read should expose ≥2 variants");
    for v in &variants {
        let via_hint = phonemes_for_draft(&[line(&format!("read({})", v.index))]);
        let flat: Vec<&str> = via_hint.into_iter().flatten().collect();
        let from_variant: Vec<&str> = v.phonemes.iter().map(|(p, _)| *p).collect();
        assert_eq!(
            flat, from_variant,
            "variant {} phonemes diverge from read({})",
            v.index, v.index
        );
    }
}

#[test]
fn cmu_variants_labels_are_uppercase_arpabet_with_stress() {
    // `read` has /riːd/ and /rɛd/ — both labels should appear, rendered
    // uppercase with a stress digit on the vowel.
    let labels: Vec<String> = cmu_variants("read").into_iter().map(|v| v.label).collect();
    assert!(
        labels.contains(&"R IY1 D".to_string()),
        "missing R IY1 D in {labels:?}"
    );
    assert!(
        labels.contains(&"R EH1 D".to_string()),
        "missing R EH1 D in {labels:?}"
    );
}

#[test]
fn cmu_variants_oov_word_has_single_rule_based_variant() {
    let variants = cmu_variants("zorglblat");
    assert_eq!(variants.len(), 1);
    assert_eq!(variants[0].index, 1);
    // Rule-based fallback carries no stress, so the label digits are all 0.
    assert!(
        variants[0].phonemes.iter().any(|(p, _)| !is_consonant(p)),
        "fallback variant should contain a vowel: {:?}",
        variants[0].phonemes
    );
}

// ---------------------------------------------------------------------
// (2) ARPAbet inventory listing
// ---------------------------------------------------------------------

#[test]
fn arpabet_symbols_include_silence_markers() {
    assert!(ARPABET_SYMBOLS.contains(&"AP"), "AP missing from inventory");
    assert!(ARPABET_SYMBOLS.contains(&"SP"), "SP missing from inventory");
}

#[test]
fn arpabet_symbols_all_validate_to_themselves() {
    // Every listed symbol must be accepted by the validator and canonical
    // to itself — the palette and the validator can't disagree.
    for &sym in ARPABET_SYMBOLS {
        assert_eq!(
            canonical_phoneme(sym),
            Some(sym),
            "inventory symbol {sym:?} failed validation"
        );
    }
}

#[test]
fn arpabet_symbols_have_no_duplicates() {
    let mut seen = std::collections::HashSet::new();
    for &sym in ARPABET_SYMBOLS {
        assert!(seen.insert(sym), "duplicate symbol {sym:?} in inventory");
    }
    // 16 vowels (incl. schwa ax) + 24 consonants + AP + SP = 42.
    assert_eq!(ARPABET_SYMBOLS.len(), 42, "unexpected inventory size");
}

// ---------------------------------------------------------------------
// (3) Public phoneme validation
// ---------------------------------------------------------------------

#[test]
fn canonical_phoneme_accepts_valid_symbols_case_insensitively() {
    assert_eq!(canonical_phoneme("ah"), Some("ah"));
    assert_eq!(canonical_phoneme("AH"), Some("ah"));
    assert_eq!(canonical_phoneme("Ng"), Some("ng"));
    assert_eq!(canonical_phoneme("ZH"), Some("zh"));
}

#[test]
fn canonical_phoneme_handles_silence_markers() {
    assert_eq!(canonical_phoneme("AP"), Some("AP"));
    assert_eq!(canonical_phoneme("SP"), Some("SP"));
    // Lowercase silence markers are NOT valid — they'd collide with real
    // phonemes if we let them through, and the convention is uppercase.
    assert_eq!(canonical_phoneme("ap"), None);
    assert_eq!(canonical_phoneme("sp"), None);
}

#[test]
fn canonical_phoneme_rejects_unknown_symbols() {
    assert_eq!(canonical_phoneme("aaa"), None);
    assert_eq!(canonical_phoneme(""), None);
    assert_eq!(canonical_phoneme("xq"), None);
}

// ---------------------------------------------------------------------
// (4) Dictionary + per-syllable override resolution & provenance
// ---------------------------------------------------------------------

#[test]
fn resolve_draft_default_provenance_is_auto() {
    let syllables = resolve_draft(&[line("break")]);
    assert_eq!(syllables.len(), 1);
    assert_eq!(syllables[0].provenance, PhonemeProvenance::Auto);
}

#[test]
fn inline_phoneme_block_provenance_is_edited() {
    // The user typing phonemes directly is the highest-precedence source.
    let syllables = resolve_draft(&[line("[hh ah l ow]")]);
    assert_eq!(syllables.len(), 1);
    assert_eq!(syllables[0].provenance, PhonemeProvenance::Edited);
}

#[test]
fn empty_dictionary_is_identical_to_plain_resolve() {
    let plain = resolve_draft(&[line("the morning break")]);
    let with_empty = resolve_draft_with_dict(&[line("the morning break")], &PhonemeDictionary::new());
    assert_eq!(plain, with_empty);
}

#[test]
fn dictionary_overrides_cmu_for_matching_word() {
    // Replace CMU's "break" (b r ey k) with a custom pronunciation.
    let mut dict: PhonemeDictionary = HashMap::new();
    dict.insert("break".to_string(), vec!["b", "r", "aa", "k"]);
    let syllables = resolve_draft_with_dict(&[line("break")], &dict);
    assert_eq!(syllables.len(), 1);
    assert_eq!(syllables[0].phonemes, vec!["b", "r", "aa", "k"]);
    assert_eq!(syllables[0].provenance, PhonemeProvenance::Dict);
}

#[test]
fn dictionary_only_affects_listed_words() {
    let mut dict: PhonemeDictionary = HashMap::new();
    dict.insert("break".to_string(), vec!["b", "r", "aa", "k"]);
    let syllables = resolve_draft_with_dict(&[line("the break")], &dict);
    // "the" is untouched → Auto; "break" is the dictionary hit → Dict.
    assert_eq!(syllables[0].phonemes, vec!["dh", "ax"]);
    assert_eq!(syllables[0].provenance, PhonemeProvenance::Auto);
    assert_eq!(syllables[1].provenance, PhonemeProvenance::Dict);
}

#[test]
fn dictionary_phonemes_split_across_syllables() {
    // A two-syllable lyric (·) re-splits the dictionary's flat list just
    // like the CMU path does.
    let mut dict: PhonemeDictionary = HashMap::new();
    dict.insert("lilia".to_string(), vec!["l", "ih", "l", "iy", "ah"]);
    let chunks: Vec<Vec<&str>> = resolve_draft_with_dict(&[line("li\u{00B7}li\u{00B7}a")], &dict)
        .into_iter()
        .map(|s| s.phonemes)
        .collect();
    assert_eq!(chunks.len(), 3, "expected 3 syllable chunks");
    let flat: Vec<&str> = chunks.into_iter().flatten().collect();
    assert_eq!(flat, vec!["l", "ih", "l", "iy", "ah"]);
}

#[test]
fn empty_overrides_is_identical_to_plain_assign() {
    let syllables = resolve_draft(&[line("morning")]);
    let ann = vec![String::new(), "+".to_string()];
    let plain = assign_syllables_to_notes(&syllables, &ann, 2);
    let with_empty =
        assign_syllables_to_notes_with(&syllables, &ann, 2, &SyllableOverrides::new());
    assert_eq!(plain, with_empty);
}

#[test]
fn per_syllable_override_replaces_phonemes_and_marks_edited() {
    let syllables = resolve_draft(&[line("break")]);
    let ann = vec![String::new()];
    let mut overrides: SyllableOverrides = HashMap::new();
    overrides.insert(0, vec!["b", "r", "ih", "k"]);
    let assigned = assign_syllables_to_notes_with(&syllables, &ann, 1, &overrides);
    assert_eq!(assigned.len(), 1);
    assert_eq!(assigned[0].phonemes, vec!["b", "r", "ih", "k"]);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Edited);
}

#[test]
fn override_takes_precedence_over_dictionary() {
    // Dictionary supplies one pronunciation; a per-syllable override on
    // the same syllable wins (override > dictionary > CMU-auto).
    let mut dict: PhonemeDictionary = HashMap::new();
    dict.insert("break".to_string(), vec!["b", "r", "aa", "k"]);
    let syllables = resolve_draft_with_dict(&[line("break")], &dict);
    assert_eq!(syllables[0].provenance, PhonemeProvenance::Dict);

    let mut overrides: SyllableOverrides = HashMap::new();
    overrides.insert(0, vec!["b", "r", "eh", "k"]);
    let assigned = assign_syllables_to_notes_with(&syllables, &[String::new()], 1, &overrides);
    assert_eq!(assigned[0].phonemes, vec!["b", "r", "eh", "k"]);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Edited);
}

#[test]
fn assigned_provenance_defaults_to_auto_without_hooks() {
    let syllables = resolve_draft(&[line("break")]);
    let assigned = assign_syllables_to_notes(&syllables, &[String::new()], 1);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Auto);
}

#[test]
fn dict_provenance_flows_through_to_assigned() {
    let mut dict: PhonemeDictionary = HashMap::new();
    dict.insert("break".to_string(), vec!["b", "r", "aa", "k"]);
    let syllables = resolve_draft_with_dict(&[line("break")], &dict);
    let assigned = assign_syllables_to_notes(&syllables, &[String::new()], 1);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Dict);
}

#[test]
fn slur_note_inherits_overridden_vowel_and_provenance() {
    // One syllable + a slur note. The slur should hold the *overridden*
    // vowel and inherit the Edited provenance.
    let syllables = resolve_draft(&[line("sun")]);
    let ann = vec![String::new(), "+".to_string()];
    let mut overrides: SyllableOverrides = HashMap::new();
    overrides.insert(0, vec!["s", "ao", "n"]); // swap the vowel to `ao`
    let assigned = assign_syllables_to_notes_with(&syllables, &ann, 2, &overrides);
    assert_eq!(assigned.len(), 2);
    assert!(assigned[1].is_slur);
    assert_eq!(assigned[1].phonemes, vec!["ao"], "slur should hold edited vowel");
    assert_eq!(assigned[1].provenance, PhonemeProvenance::Edited);
}

#[test]
fn slur_provenance_is_auto_for_plain_resolution() {
    let syllables = resolve_draft(&[line("sun")]);
    let ann = vec![String::new(), "+".to_string()];
    let assigned = assign_syllables_to_notes(&syllables, &ann, 2);
    assert_eq!(assigned[1].provenance, PhonemeProvenance::Auto);
    // Stress still propagates as before.
    assert_eq!(assigned[1].stress, SyllableStress::Primary);
}
