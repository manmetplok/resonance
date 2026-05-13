//! Tests for the grapheme-to-phoneme module. Verifies that the CMU
//! dictionary loads, common English words come out with the expected
//! pronunciations, and the multi-syllable word splitter produces
//! sensible per-note chunks.

use resonance_music_theory::g2p::{is_consonant, phonemes_for_draft, CONSONANTS};
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
        ("the", vec!["dh", "ah"]),
        ("they", vec!["dh", "ey"]),
        ("break", vec!["b", "r", "ey", "k"]),
        ("glass", vec!["g", "l", "ae", "s"]),
        ("houses", vec!["hh", "aw", "s", "ah", "z"]),
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
fn punctuation_is_stripped() {
    // Apostrophes are kept (CMU has "don't"); other punctuation gets
    // dropped without affecting the word boundary.
    let chunks = phonemes_for_draft(&[line("don't, break")]);
    let flat: Vec<&str> = chunks.into_iter().flatten().collect();
    assert_eq!(flat, vec!["d", "ow", "n", "t", "b", "r", "ey", "k"]);
}
