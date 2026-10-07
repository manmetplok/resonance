//! Lyric-draft markers and syllable sources (spec
//! `vocal-mcp-control-and-quality.md` §6 and §8): the `_` melisma token
//! and its `+` alias, the `AP` breath token, exact user `·`
//! breaks with their mismatch warning, the per-syllable `G2pSource`, and
//! the corpus-wide CMU guarantee.

use std::collections::HashMap;

use resonance_music_theory::g2p::{
    assign_syllables_to_notes, assign_syllables_to_notes_with, canonical_phoneme, is_slur_lyric,
    is_slur_token, note_unit_count, resolve_draft, resolve_draft_with_dict, word_g2p_source, G2pSource,
    PhonemeDictionary, PhonemeProvenance, SyllableOverrides,
};
use resonance_music_theory::{count_syllables, lyric_corpus_lines, LyricLine};

fn line(text: &str) -> LyricLine {
    LyricLine {
        n: 1,
        rhyme: 'A',
        syllables: 0,
        text: text.into(),
        locked: false,
    }
}

fn no_annotations(n: usize) -> Vec<String> {
    vec![String::new(); n]
}

// ---------------------------------------------------------------------
// Melisma
// ---------------------------------------------------------------------

#[test]
fn underscore_holds_the_previous_syllable_and_counts_agree() {
    let draft = [line("love _ _")];
    let syllables = resolve_draft(&draft);
    assert_eq!(syllables.len(), 1, "`_` is not a sung syllable");
    assert_eq!(syllables[0].label, "love");
    assert_eq!(syllables[0].slurs_after, 2);
    assert_eq!(note_unit_count(&syllables), 3);
    assert_eq!(count_syllables("love _ _"), 3, "count_syllables must agree");

    let notes = assign_syllables_to_notes(&syllables, &no_annotations(3), 3);
    assert!(!notes[0].is_slur);
    // The final consonant is sung at the END of the hold, not on the
    // first note: `l ah` · `ah` · `ah v`.
    assert_eq!(notes[0].phonemes, vec!["l", "ah"]);
    assert_eq!(notes[1].phonemes, vec!["ah"], "a hold sings the held vowel");
    assert_eq!(notes[2].phonemes, vec!["ah", "v"], "the coda closes the hold");
    assert!(!notes[0].is_word_end && !notes[1].is_word_end);
    assert!(notes[2].is_word_end, "the word ends where its coda is sung");
    for n in &notes[1..] {
        assert!(n.is_slur);
        assert_eq!(n.syllable_index, 0);
        assert_eq!(n.source, G2pSource::Cmu, "a hold inherits the held source");
    }
}

#[test]
fn the_coda_of_an_overridden_held_syllable_moves_too() {
    let syllables = resolve_draft(&[line("love _ me")]);
    let mut overrides: SyllableOverrides = HashMap::new();
    overrides.insert(
        0,
        ["l", "ow", "v", "z"].map(|p| canonical_phoneme(p).unwrap()).to_vec(),
    );
    let notes = assign_syllables_to_notes_with(&syllables, &no_annotations(3), 3, &overrides);
    assert_eq!(notes[0].phonemes, vec!["l", "ow"]);
    assert_eq!(notes[1].phonemes, vec!["ow", "v", "z"]);
    assert_eq!(notes[2].phonemes, vec!["m", "iy"], "the next syllable is untouched");
    assert!(notes[1].is_word_end && notes[2].is_word_end);
}

#[test]
fn an_unheld_syllable_keeps_its_coda() {
    let syllables = resolve_draft(&[line("love me")]);
    let notes = assign_syllables_to_notes(&syllables, &no_annotations(2), 2);
    assert_eq!(notes[0].phonemes, vec!["l", "ah", "v"]);
    assert!(notes[0].is_word_end);
}

#[test]
fn underscore_does_not_shift_later_syllable_indices() {
    let syllables = resolve_draft(&[line("love _ me")]);
    assert_eq!(syllables.len(), 2);
    let notes = assign_syllables_to_notes(&syllables, &no_annotations(3), 3);
    assert_eq!(notes[2].label, "me");
    assert_eq!(notes[2].syllable_index, 1);
    assert_eq!(count_syllables("love _ me"), 3);
}

/// Auto-syllabify copies letterless tokens through untouched. It used to
/// count a one-character token's punctuation as both lead and trail —
/// a subtraction overflow in debug, and `_` doubled to `__` in release.
#[test]
fn auto_syllabify_keeps_markers_and_punctuation_tokens() {
    use resonance_music_theory::g2p::auto_syllabify_text;
    assert_eq!(auto_syllabify_text("love _ _"), "love _ _");
    assert_eq!(auto_syllabify_text("hold + AP on & - so"), "hold + AP on & - so");
    assert_eq!(auto_syllabify_text("re·mem·ber _"), "re·mem·ber _");
}

#[test]
fn plus_is_a_slur_alias() {
    let syllables = resolve_draft(&[line("love + +")]);
    assert_eq!(syllables.len(), 1);
    assert_eq!(syllables[0].slurs_after, 2);
    assert_eq!(count_syllables("love + +"), 3);
}

/// A spaced `-` in the draft is a dash in prose, not a hold: reading it
/// as a melisma silently added a note.
#[test]
fn a_spaced_dash_is_punctuation_not_a_hold() {
    let syllables = resolve_draft(&[line("wait - no")]);
    let labels: Vec<&str> = syllables.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, vec!["wait", "no"]);
    assert!(syllables.iter().all(|s| s.slurs_after == 0), "no hold");
    assert_eq!(note_unit_count(&syllables), 2);
    assert_eq!(count_syllables("wait - no"), 2);
    // The per-note side table still reads a lone `-` as a slur.
    assert!(is_slur_lyric("-") && is_slur_lyric(" + ") && is_slur_lyric("_"));
    assert!(!is_slur_token("-"));
}

#[test]
fn leading_underscore_has_nothing_to_hold_and_is_dropped() {
    let syllables = resolve_draft(&[line("_ love")]);
    assert_eq!(syllables.len(), 1);
    assert_eq!(syllables[0].slurs_after, 0);
    assert_eq!(note_unit_count(&syllables), 1);
    assert_eq!(count_syllables("_ love"), 1);
}

#[test]
fn per_note_slur_on_a_draft_hold_is_the_same_hold() {
    let syllables = resolve_draft(&[line("love _ me")]);
    let mut annotations = no_annotations(3);
    annotations[1] = "+".into();
    let notes = assign_syllables_to_notes(&syllables, &annotations, 3);
    assert!(notes[1].is_slur);
    assert_eq!(notes[2].label, "me", "the annotation must not add a second hold");
}

/// A per-note label on a note the draft would hold names the syllable
/// that note starts: it ends the pending holds and takes the next
/// syllable rather than being swallowed as a hold.
#[test]
fn a_label_annotation_ends_pending_draft_holds() {
    let syllables = resolve_draft(&[line("love _ _ me")]);
    let mut annotations = no_annotations(3);
    annotations[1] = "ME!".into();
    let notes = assign_syllables_to_notes(&syllables, &annotations, 3);
    assert!(!notes[1].is_slur, "the label ends the hold");
    assert_eq!(notes[1].label, "ME!");
    assert_eq!(notes[1].syllable_index, 1);
    assert_eq!(notes[1].phonemes, vec!["m", "iy"]);
    assert_eq!(notes[0].phonemes, vec!["l", "ah", "v"], "no hold left to carry the coda");
    assert!(notes[2].label.is_empty(), "the draft is exhausted");
}

#[test]
fn hold_after_a_multi_syllable_word_holds_its_last_syllable() {
    let syllables = resolve_draft(&[line("for·get _")]);
    assert_eq!(syllables.len(), 2);
    assert_eq!(syllables[1].slurs_after, 1);
    let notes = assign_syllables_to_notes(&syllables, &no_annotations(3), 3);
    assert!(notes[2].is_slur);
    assert_eq!(notes[1].phonemes, vec!["g", "eh"]);
    assert_eq!(notes[2].phonemes, vec!["eh", "t"]);
    assert_eq!(count_syllables("for·get _"), 3);
}

// ---------------------------------------------------------------------
// Breath
// ---------------------------------------------------------------------

#[test]
fn ap_takes_no_note_and_marks_the_next_syllable() {
    let draft = [line("hold AP on")];
    let syllables = resolve_draft(&draft);
    let labels: Vec<&str> = syllables.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, vec!["hold", "on"], "AP is never sung as a word");
    assert!(!syllables[0].breath_before);
    assert!(syllables[1].breath_before);
    assert_eq!(note_unit_count(&syllables), 2);
    assert_eq!(count_syllables("hold AP on"), 2);

    let notes = assign_syllables_to_notes(&syllables, &no_annotations(2), 2);
    assert!(!notes[0].breath_before);
    assert!(notes[1].breath_before);
}

#[test]
fn ap_carries_across_a_line_break_and_is_dropped_at_the_end() {
    let syllables = resolve_draft(&[line("hold AP"), line("on AP")]);
    assert_eq!(syllables.len(), 2);
    assert!(syllables[1].breath_before);
    assert_eq!(count_syllables("hold AP"), 1);
    assert_eq!(count_syllables("on AP"), 1);
}

#[test]
fn lowercase_ap_is_a_word() {
    let syllables = resolve_draft(&[line("hold ap on")]);
    assert_eq!(syllables.len(), 3);
    assert_eq!(syllables[1].label, "ap");
    assert!(!syllables[2].breath_before);
    assert_eq!(count_syllables("hold ap on"), 3);
}

#[test]
fn bracketed_ap_stays_an_inline_phoneme_block() {
    let syllables = resolve_draft(&[line("hold [AP] on")]);
    assert_eq!(syllables.len(), 3, "[AP] occupies its own note");
    assert_eq!(syllables[1].phonemes, vec!["AP"]);
    assert!(!syllables[2].breath_before);
}

// ---------------------------------------------------------------------
// Exact user breaks
// ---------------------------------------------------------------------

#[test]
fn user_breaks_set_labels_and_phoneme_split() {
    // CMU: forget = F ER0 G EH1 T; the break goes before the `g`.
    let s = resolve_draft(&[line("for·get")]);
    let labels: Vec<&str> = s.iter().map(|x| x.label.as_str()).collect();
    assert_eq!(labels, vec!["for", "get"]);
    assert_eq!(s[0].phonemes, vec!["f", "er"]);
    assert_eq!(s[1].phonemes, vec!["g", "eh", "t"]);
    assert!(s.iter().all(|x| !x.syllable_warning && x.source == G2pSource::Cmu));

    // The old corpus misspelling is not a CMU word: the rules read it as
    // three vowels (`f ow r eh g eh t`), which two pieces cannot hold, so
    // it is flagged — while its labels stay exactly the written pieces.
    let s = resolve_draft(&[line("fore·get")]);
    let labels: Vec<&str> = s.iter().map(|x| x.label.as_str()).collect();
    assert_eq!(labels, vec!["fore", "get"]);
    assert!(s.iter().all(|x| x.source == G2pSource::Rules && x.syllable_warning));
}

#[test]
fn user_break_onset_follows_the_written_consonant_letters() {
    // CMU: function = F AH1 NG K SH AH0 N. `func·tion`: `tion` starts with
    // one consonant letter, so the onset is `sh` and `ng k` stay as coda.
    let s = resolve_draft(&[line("func·tion")]);
    assert_eq!(s[0].phonemes, vec!["f", "ah", "ng", "k"]);
    assert_eq!(s[1].phonemes, vec!["sh", "ax", "n"]);
    // Moving the break moves the phonemes with it: `fun·ction` hands `ct`
    // (two letters) to the second piece.
    let s = resolve_draft(&[line("fun·ction")]);
    assert_eq!(s[0].label, "fun");
    assert_eq!(s[0].phonemes, vec!["f", "ah", "ng"]);
    assert_eq!(s[1].phonemes, vec!["k", "sh", "ax", "n"]);
    // A digraph counts once: `ma·chine` (M AH0 SH IY1 N).
    let s = resolve_draft(&[line("ma·chine")]);
    assert_eq!(s[0].phonemes, vec!["m", "ax"]);
    assert_eq!(s[1].phonemes, vec!["sh", "iy", "n"]);
    // A vowel-initial piece takes no onset: `be·cause`'s `cause` takes `k`.
    let s = resolve_draft(&[line("be·cause")]);
    assert_eq!(s[1].phonemes[0], "k");
}

fn split(text: &str) -> Vec<Vec<&'static str>> {
    let s = resolve_draft(&[line(text)]);
    assert!(s.iter().all(|x| !x.syllable_warning), "{text} warned: {s:?}");
    s.into_iter().map(|x| x.phonemes).collect()
}

/// The onset rule weighs both sides of the break:
/// `max(leading(next), cluster - trailing(prev))`, clamped to the cluster.
#[test]
fn user_break_onset_counts_the_previous_piece_coda_too() {
    // `ery` starts with a vowel, but `ev` ends in one consonant, so the
    // `r` goes with the next piece.
    assert_eq!(
        split("ev·ery·thing"),
        vec![vec!["eh", "v"], vec!["r", "iy"], vec!["th", "ih", "ng"]]
    );
    assert_eq!(split("for·get(2)"), vec![vec!["f", "ao", "r"], vec!["g", "eh", "t"]]);
    assert_eq!(split("for·get"), vec![vec!["f", "er"], vec!["g", "eh", "t"]]);
    let remember = split("re·mem·ber");
    assert_eq!(remember[1].last(), Some(&"m"));
    assert_eq!(remember[2], vec!["b", "er"]);
    assert_eq!(split("an·y"), vec![vec!["eh", "n"], vec!["iy"]]);
    assert_eq!(split("e·ver"), vec![vec!["eh"], vec!["v", "er"]]);
    // A silent final `e` does not hide the coda.
    assert_eq!(split("love·ly"), vec![vec!["l", "ah", "v"], vec!["l", "iy"]]);
}

/// Without a `(N)` hint, the piece count picks among CMU's variants.
#[test]
fn piece_count_selects_the_matching_cmu_variant() {
    assert_eq!(split("ev·ery"), vec![vec!["eh", "v"], vec!["r", "iy"]]);
    assert_eq!(split("ev·er·y"), vec![vec!["eh", "v"], vec!["er"], vec!["iy"]]);
    // An explicit hint is never second-guessed: variant 1 has three vowels.
    let s = resolve_draft(&[line("ev·ery(1)")]);
    assert!(s.iter().all(|x| x.syllable_warning));
}

#[test]
fn piece_count_mismatch_falls_back_and_warns() {
    // `words` has one vowel nucleus but is written as two pieces.
    let s = resolve_draft(&[line("the word·s")]);
    assert_eq!(s.len(), 3, "the note count still follows the pieces");
    assert!(!s[0].syllable_warning);
    assert!(s[1].syllable_warning && s[2].syllable_warning);
    assert_eq!(s[1].label, "word");
    assert_eq!(s[2].label, "s");
    let notes = assign_syllables_to_notes(&s, &no_annotations(3), 3);
    assert!(notes[1].syllable_warning);
}

#[test]
fn count_syllables_agrees_with_the_resolver() {
    for (text, expected) in [
        ("re-so-lu-tion", 4),
        ("[l ih · l iy]", 2),
        ("you & me", 2),
        ("&", 1),
        ("love _ _", 3),
        ("hold AP on", 2),
        ("wait - no", 2),
        ("the long, the qui·et, the un·done", 8),
    ] {
        assert_eq!(count_syllables(text), expected, "{text}");
        let resolved = note_unit_count(&resolve_draft(&[line(text)]));
        assert_eq!(resolved.max(1), expected as usize, "resolver on {text}");
    }
}

#[test]
fn unbroken_words_never_warn() {
    let s = resolve_draft(&[line("fire resolution")]);
    assert!(s.iter().all(|x| !x.syllable_warning));
}

// ---------------------------------------------------------------------
// Pronunciation source
// ---------------------------------------------------------------------

#[test]
fn source_distinguishes_cmu_rules_dict_and_edits() {
    let s = resolve_draft(&[line("hello xyzzyq [hh ah]")]);
    assert_eq!(s[0].source, G2pSource::Cmu);
    assert_eq!(s[0].provenance, PhonemeProvenance::Auto);
    assert_eq!(s[1].source, G2pSource::Rules, "out-of-dictionary word");
    assert_eq!(s[1].provenance, PhonemeProvenance::Auto);
    assert_eq!(s[2].source, G2pSource::Edited);

    let mut dict = PhonemeDictionary::new();
    dict.insert(
        "hello".into(),
        vec![canonical_phoneme("hh").unwrap(), canonical_phoneme("ow").unwrap()],
    );
    let s = resolve_draft_with_dict(&[line("hello")], &dict);
    assert_eq!(s[0].source, G2pSource::ProjectDict);
    assert_eq!(s[0].provenance, PhonemeProvenance::Dict);

    let s = resolve_draft(&[line("hello _")]);
    let mut overrides: SyllableOverrides = HashMap::new();
    overrides.insert(0, vec![canonical_phoneme("ow").unwrap()]);
    let notes = assign_syllables_to_notes_with(&s, &no_annotations(2), 2, &overrides);
    assert_eq!(notes[0].source, G2pSource::Edited);
    assert_eq!(notes[1].source, G2pSource::Edited, "the hold inherits the edit");
}

#[test]
fn word_g2p_source_reports_cmu_or_rules() {
    assert_eq!(word_g2p_source("Hello,"), G2pSource::Cmu);
    assert_eq!(word_g2p_source("for·get"), G2pSource::Cmu);
    assert_eq!(word_g2p_source("xyzzyq"), G2pSource::Rules);
}

#[test]
fn g2p_source_serialises_snake_case() {
    let names: Vec<String> = [
        G2pSource::Edited,
        G2pSource::ProjectDict,
        G2pSource::BankDict,
        G2pSource::Cmu,
        G2pSource::Rules,
    ]
    .iter()
    .map(|s| {
        let json = serde_json::to_string(s).unwrap();
        assert_eq!(json, format!("\"{}\"", s.as_str()));
        s.as_str().to_string()
    })
    .collect();
    assert_eq!(names, ["edited", "project_dict", "bank_dict", "cmu", "rules"]);
}

// ---------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------

#[test]
fn every_corpus_word_resolves_through_cmu() {
    let mut misses = Vec::new();
    let mut warnings = Vec::new();
    for text in lyric_corpus_lines() {
        for s in resolve_draft(&[line(text)]) {
            if s.source != G2pSource::Cmu {
                misses.push(format!("{:?} in {text:?}", s.label));
            }
            if s.syllable_warning {
                warnings.push(format!("{:?} in {text:?}", s.label));
            }
        }
    }
    assert!(misses.is_empty(), "corpus syllables off CMU: {misses:#?}");
    assert!(warnings.is_empty(), "corpus breaks off CMU's vowels: {warnings:#?}");
}

#[test]
fn corpus_has_no_misspelled_forget() {
    assert!(lyric_corpus_lines().iter().any(|l| l.contains("for·get")));
    assert!(!lyric_corpus_lines().iter().any(|l| l.contains("fore·get")));
}
