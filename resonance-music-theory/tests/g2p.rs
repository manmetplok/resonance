//! Tests for the grapheme-to-phoneme module. Verifies that the CMU
//! dictionary loads, common English words come out with the expected
//! pronunciations, and the multi-syllable word splitter produces
//! sensible per-note chunks.

use resonance_music_theory::g2p::{
    assign_syllables_to_notes, auto_syllabify_text, cmu_syllable_count, cmu_variant_count,
    is_consonant, phonemes_for_draft, resolve_draft, syllabify_word, SyllableStress, CONSONANTS,
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

#[test]
fn consonants_set_has_no_vowels() {
    // Sanity: nothing in CONSONANTS overlaps the ARPAbet vowel set.
    let vowels = [
        "aa", "ae", "ah", "ao", "aw", "ay", "eh", "er", "ey", "ih", "iy", "ow", "oy", "uh", "uw",
    ];
    for v in vowels {
        assert!(
            !CONSONANTS.contains(&v),
            "vowel {v} mis-classified as consonant"
        );
    }
}

#[test]
fn is_consonant_classification() {
    assert!(is_consonant("b"));
    assert!(is_consonant("ng"));
    assert!(is_consonant("zh"));
    assert!(!is_consonant("ah"));
    assert!(!is_consonant("ay"));
    assert!(!is_consonant("AP")); // SP/AP are silence markers — neither
}

#[test]
fn cmu_lookup_known_words() {
    // The CMU lookup is the whole reason this module exists — verify
    // the vendored dict is being loaded and these high-confidence
    // pronunciations come out exactly as CMU specifies (stress
    // numbers dropped).
    let cases = [
        // `the` ends in CMU's AH0 (unstressed) — emitted as the
        // schwa `ax`, the natural pronunciation of function words.
        ("the", vec!["dh", "ax"]),
        ("they", vec!["dh", "ey"]),
        ("break", vec!["b", "r", "ey", "k"]),
        ("glass", vec!["g", "l", "ae", "s"]),
        // `houses` second syllable is also AH0 → ax.
        ("houses", vec!["hh", "aw", "s", "ax", "z"]),
    ];
    for (word, expected) in cases {
        let got = phonemes_for_draft(&[line(word)]);
        let flat: Vec<&str> = got.into_iter().flatten().collect();
        assert_eq!(flat, expected, "G2P mismatch for {word:?}");
    }
}

#[test]
fn multi_syllable_split() {
    // `re·mem·ber` is one CMU word but three syllables in the draft;
    // the splitter must produce three chunks. Each chunk should
    // contain at least one vowel.
    let chunks = phonemes_for_draft(&[line("re\u{00B7}mem\u{00B7}ber")]);
    assert_eq!(chunks.len(), 3, "expected 3 syllable chunks for re·mem·ber");
    for (i, chunk) in chunks.iter().enumerate() {
        assert!(
            chunk.iter().any(|p| !is_consonant(p)),
            "syllable {i} ({chunk:?}) has no vowel"
        );
    }
    // Sanity: concatenating the chunks gets us back the whole-word
    // phoneme list (no phonemes lost in the split).
    let flat: Vec<&str> = chunks.into_iter().flatten().collect();
    assert_eq!(flat, vec!["r", "ih", "m", "eh", "m", "b", "er"]);
}

#[test]
fn multi_word_line_preserves_order() {
    // Word boundaries (whitespace) are honoured; the output has one
    // chunk per syllable in left-to-right order.
    let chunks = phonemes_for_draft(&[line("they break")]);
    let flat: Vec<&str> = chunks.into_iter().flatten().collect();
    assert_eq!(flat, vec!["dh", "ey", "b", "r", "ey", "k"]);
}

#[test]
fn fallback_for_unknown_words_still_emits_vowel() {
    // Made-up word — CMU won't have it, rule-based fallback fires.
    // Output should still be non-empty and contain at least one vowel.
    let chunks = phonemes_for_draft(&[line("zorglblat")]);
    let flat: Vec<&str> = chunks.into_iter().flatten().collect();
    assert!(!flat.is_empty(), "empty fallback output for unknown word");
    assert!(
        flat.iter().any(|p| !is_consonant(p)),
        "fallback produced no vowel: {flat:?}"
    );
}

#[test]
fn empty_lyrics_produces_no_chunks() {
    let chunks = phonemes_for_draft(&[line("")]);
    assert!(chunks.is_empty());
}

#[test]
fn cmu_syllable_count_matches_vowel_phonemes() {
    // Single-vowel words.
    assert_eq!(cmu_syllable_count("the"), 1);
    assert_eq!(cmu_syllable_count("break"), 1);
    assert_eq!(cmu_syllable_count("glass"), 1);
    // Two vowels in CMU.
    assert_eq!(cmu_syllable_count("morning"), 2);
    assert_eq!(cmu_syllable_count("houses"), 2);
    // Three vowels.
    assert_eq!(cmu_syllable_count("remember"), 3);
    assert_eq!(cmu_syllable_count("library"), 3);
    assert_eq!(cmu_syllable_count("everything"), 3);
    // Empty / weird input always returns at least 1.
    assert_eq!(cmu_syllable_count(""), 1);
    assert_eq!(cmu_syllable_count("..."), 1);
}

#[test]
fn syllabify_word_inserts_correct_dot_count() {
    // No-op when target ≤ existing.
    assert_eq!(syllabify_word("the", 1), "the");
    assert_eq!(syllabify_word("hou\u{00B7}ses", 2), "hou\u{00B7}ses");
    // Inserts up to the target.
    let out = syllabify_word("morning", 2);
    assert_eq!(out.matches('\u{00B7}').count(), 1);
    let out = syllabify_word("library", 3);
    assert_eq!(out.matches('\u{00B7}').count(), 2);
    let out = syllabify_word("everything", 3);
    assert_eq!(out.matches('\u{00B7}').count(), 2);
}

#[test]
fn syllabify_word_preserves_existing_dots() {
    // User-added extra dots (intentional melisma) stay.
    assert_eq!(syllabify_word("ah\u{00B7}ah\u{00B7}ah", 2), "ah\u{00B7}ah\u{00B7}ah");
}

#[test]
fn auto_syllabify_text_fixes_under_dotted_words() {
    let out = auto_syllabify_text("everything is morning");
    // `everything` (3 CMU syl) and `morning` (2 syl) get dots; `is` (1 syl) doesn't.
    assert!(
        out.matches('\u{00B7}').count() >= 3,
        "expected ≥3 dots, got {:?}",
        out
    );
}

#[test]
fn auto_syllabify_text_keeps_punctuation() {
    // Punctuation around a word should survive the round-trip.
    let out = auto_syllabify_text("hello, library!");
    assert!(out.contains(','), "comma lost in {:?}", out);
    assert!(out.ends_with("!"), "trailing `!` lost in {:?}", out);
    // hello = 2 syl, library = 3 syl → at least 3 dots total.
    assert!(
        out.matches('\u{00B7}').count() >= 3,
        "expected ≥3 dots, got {:?}",
        out
    );
}

#[test]
fn inline_phoneme_block_overrides_cmu() {
    // `[hh ah l ow]` should override CMU's "hello" pronunciation
    // and emit exactly those phonemes as one syllable.
    let chunks = phonemes_for_draft(&[line("[hh ah l ow]")]);
    assert_eq!(chunks, vec![vec!["hh", "ah", "l", "ow"]]);
}

#[test]
fn inline_phoneme_block_with_inner_syllable_marks() {
    // `·` inside the bracket splits the override into multiple syllables.
    let chunks = phonemes_for_draft(&[line("[l ih \u{00B7} l iy \u{00B7} ah]")]);
    assert_eq!(
        chunks,
        vec![
            vec!["l", "ih"],
            vec!["l", "iy"],
            vec!["ah"],
        ]
    );
}

#[test]
fn inline_phoneme_block_mixed_with_cmu_words() {
    // Override a single tricky word inline; surrounding CMU words still work.
    let chunks = phonemes_for_draft(&[line("the [jh oh r ih t] sings")]);
    let flat: Vec<&str> = chunks.into_iter().flatten().collect();
    // "the" → dh ax (schwa); override → jh + r + ih + t (the bogus
    // "oh" is dropped as not-a-symbol); "sings" → s ih ng z.
    assert!(flat.starts_with(&["dh", "ax"]));
    assert!(flat.ends_with(&["s", "ih", "ng", "z"]));
    // Override survives at least the valid symbols (`jh`, `r`, `ih`, `t`).
    assert!(flat.contains(&"jh"));
}

#[test]
fn inline_phoneme_block_drops_unknown_symbols() {
    // Typo / unknown phoneme is silently dropped, no panic.
    let chunks = phonemes_for_draft(&[line("[hh aaa l ow]")]);
    // "aaa" isn't valid ARPAbet; expect `hh, l, ow` (vowel-injected
    // before the last consonant by `ensure_vowel`? — no, ensure_vowel
    // only runs for word_to_phonemes; bracket path uses raw output).
    // Acceptable behaviour: drop the unknown, keep the rest.
    let flat: Vec<&str> = chunks.into_iter().flatten().collect();
    assert!(flat.contains(&"hh"));
    assert!(flat.contains(&"ow"));
    assert!(!flat.contains(&"aaa"));
}

#[test]
fn unclosed_bracket_falls_through_to_word() {
    // `[bogus` without a closing `]` shouldn't crash; the leftover
    // text is just dropped (it doesn't match any word pattern).
    let chunks = phonemes_for_draft(&[line("the [bogus phoneme stream")]);
    let flat: Vec<&str> = chunks.into_iter().flatten().collect();
    // We still get "the" and the trailing words, just no bracket-parsed
    // phoneme block. No panic is the main success condition.
    assert!(flat.contains(&"dh"));
}

#[test]
fn cmu_variant_count_reports_available_alternates() {
    // CMU has multiple pronunciations for these.
    assert!(cmu_variant_count("read") >= 2, "read should have ≥2 variants");
    assert!(cmu_variant_count("live") >= 2, "live should have ≥2 variants");
    // OOV word always reports 1.
    assert_eq!(cmu_variant_count("xyzzy"), 1);
    // Some single-syllable words also have multiple weak/strong reductions
    // in CMU (e.g. `the` lists `dh ah` and `dh iy`); just verify ≥ 1.
    assert!(cmu_variant_count("the") >= 1);
}

#[test]
fn multi_pronunciation_hint_picks_alternate() {
    // `live` defaults to /laɪv/ (adjective). `live(2)` should pick a
    // different variant — the verb /lɪv/ — which CMU encodes
    // separately.
    let default = phonemes_for_draft(&[line("live")]);
    let alternate = phonemes_for_draft(&[line("live(2)")]);
    assert_ne!(
        default, alternate,
        "live(2) should produce a different phoneme stream than `live`"
    );
}

#[test]
fn multi_pronunciation_out_of_range_clamps() {
    // `the(99)` clamps to the last available variant (no panic).
    let chunks = phonemes_for_draft(&[line("the(99)")]);
    assert!(!chunks.is_empty());
    assert!(chunks[0].iter().any(|p| !is_consonant(p)));
}

#[test]
fn variant_hint_does_not_eat_punctuation_only_paren() {
    // `morning(` (no closing paren) should not be treated as a hint —
    // it's just a malformed word with a stray `(`.
    let chunks = phonemes_for_draft(&[line("morning(")]);
    let flat: Vec<&str> = chunks.into_iter().flatten().collect();
    // We still get morning's phonemes (`m ao r n ih ng`); the trailing
    // `(` is stripped as punctuation.
    assert!(flat.contains(&"m"));
    assert!(flat.contains(&"ng"));
}

#[test]
fn stress_marks_propagate_from_cmu() {
    // CMU encodes `computer` as K AH0 M P Y UW1 T ER0 — schwa, then
    // primary on the UW, then unstressed ER. After splitting into 3
    // syllables (`com·pu·ter`) we expect: None, Primary, None.
    let syllables = resolve_draft(&[line("com\u{00B7}pu\u{00B7}ter")]);
    assert_eq!(syllables.len(), 3, "expected 3 syllables for com·pu·ter");
    assert_eq!(syllables[0].stress, SyllableStress::None);
    assert_eq!(syllables[1].stress, SyllableStress::Primary);
    assert_eq!(syllables[2].stress, SyllableStress::None);
}

#[test]
fn stress_marks_are_secondary_for_some_words() {
    // CMU encodes `university` as Y UW2 N IH0 V ER1 S AH0 T IY0 —
    // secondary on the leading `u`, primary on `ver`, none elsewhere.
    let syllables = resolve_draft(&[line("u\u{00B7}ni\u{00B7}ver\u{00B7}si\u{00B7}ty")]);
    assert_eq!(syllables.len(), 5);
    assert_eq!(syllables[0].stress, SyllableStress::Secondary);
    assert_eq!(syllables[2].stress, SyllableStress::Primary);
    assert!(matches!(
        syllables[1].stress,
        SyllableStress::None
    ));
}

#[test]
fn stress_is_none_for_inline_phoneme_blocks() {
    // User-typed override carries no stress.
    let syllables = resolve_draft(&[line("[hh ah l ow]")]);
    assert_eq!(syllables.len(), 1);
    assert_eq!(syllables[0].stress, SyllableStress::None);
}

#[test]
fn slur_notes_inherit_previous_syllable_stress() {
    // Two notes, one syllable + one slur. The slur note's stress
    // should match the syllable it's holding.
    let syllables = resolve_draft(&[line("sun")]);
    let ann = vec![String::new(), "+".to_string()];
    let assigned = assign_syllables_to_notes(&syllables, &ann, 2);
    assert_eq!(assigned.len(), 2);
    assert!(!assigned[0].is_slur);
    assert!(assigned[1].is_slur);
    // `sun` (S AH1 N) is primary stress on the AH.
    assert_eq!(assigned[0].stress, SyllableStress::Primary);
    assert_eq!(assigned[1].stress, assigned[0].stress);
}

#[test]
fn stress_velocity_factor_orders_correctly() {
    // Primary > Secondary > None — the modulation must keep that order
    // so primary-stress syllables always sing louder than unstressed
    // schwas under the same baseline velocity.
    assert!(
        SyllableStress::Primary.velocity_factor() > SyllableStress::Secondary.velocity_factor()
    );
    assert!(SyllableStress::Secondary.velocity_factor() > SyllableStress::None.velocity_factor());
}

#[test]
fn punctuation_is_stripped() {
    // Apostrophes are kept (CMU has "don't"); other punctuation gets
    // dropped without affecting the word boundary.
    let chunks = phonemes_for_draft(&[line("don't, break")]);
    let flat: Vec<&str> = chunks.into_iter().flatten().collect();
    assert_eq!(flat, vec!["d", "ow", "n", "t", "b", "r", "ey", "k"]);
}
