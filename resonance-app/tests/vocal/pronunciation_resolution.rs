//! Tests for the app-side pronunciation data model + resolver (design
//! #173, notes #174, todo #493).
//!
//! These pin the resolution precedence `override > project-dict >
//! global-dict > CMU-auto` and the [`PhonemeProvenance`] each layer
//! stamps, plus the small state-management helpers on
//! [`PronunciationState`]. The actual phoneme transcription is g2p's job
//! (tested there); here we only assert that the layering and provenance
//! come out right.

use std::collections::HashMap;

use resonance_app::compose::vocal_svs::{
    canonicalize_phonemes, clean_word, resolve_clip_pronunciation, DictionaryEntry,
    DictionaryScope, PhonemeProvenance, PronunciationState, SyllableOverride,
};
use resonance_app::project::{
    ProjectPronunciation, ProjectPronunciationEntry, ProjectSyllableOverride,
};
use resonance_music_theory::derive::LyricLine;

/// One-line draft from raw text. The `·` in `text` separates syllables
/// (the resolver re-splits a word's phonemes across them); the other
/// fields don't affect resolution.
fn line(text: &str) -> Vec<LyricLine> {
    vec![LyricLine {
        n: 1,
        rhyme: 'A',
        syllables: 1,
        text: text.to_string(),
        locked: false,
    }]
}

fn project(word: &str, phonemes: &[&str]) -> DictionaryEntry {
    DictionaryEntry::new(word, phonemes, DictionaryScope::Project)
}

fn global(word: &str, phonemes: &[&str]) -> DictionaryEntry {
    DictionaryEntry::new(word, phonemes, DictionaryScope::Global)
}

// ---------------------------------------------------------------------------
// Precedence + provenance
// ---------------------------------------------------------------------------

#[test]
fn bare_resolution_is_auto() {
    // No overrides, no dictionaries: the single note resolves straight
    // from CMU / rule-based g2p and reports Auto.
    let draft = line("hello");
    let assigned = resolve_clip_pronunciation(&draft, &[], 1, &HashMap::new(), &[], &[]);
    assert_eq!(assigned.len(), 1);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Auto);
    assert!(
        !assigned[0].phonemes.is_empty(),
        "auto path should still produce phonemes"
    );
}

#[test]
fn project_dictionary_beats_auto() {
    let draft = line("hello");
    let dict = vec![project("hello", &["k", "ae", "t"])];
    let assigned = resolve_clip_pronunciation(&draft, &[], 1, &HashMap::new(), &dict, &[]);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Dict);
    assert_eq!(assigned[0].phonemes, vec!["k", "ae", "t"]);
}

#[test]
fn global_dictionary_used_when_no_project_entry() {
    let draft = line("hello");
    let glob = vec![global("hello", &["g", "uh"])];
    let assigned = resolve_clip_pronunciation(&draft, &[], 1, &HashMap::new(), &[], &glob);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Dict);
    assert_eq!(assigned[0].phonemes, vec!["g", "uh"]);
}

#[test]
fn project_dictionary_beats_global() {
    // Same word in both scopes — project wins (project overlays global in
    // the merge), still reported as Dict.
    let draft = line("hello");
    let proj = vec![project("hello", &["p", "iy"])];
    let glob = vec![global("hello", &["g", "uh"])];
    let assigned = resolve_clip_pronunciation(&draft, &[], 1, &HashMap::new(), &proj, &glob);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Dict);
    assert_eq!(assigned[0].phonemes, vec!["p", "iy"]);
}

#[test]
fn override_beats_dictionary_and_auto() {
    let draft = line("hello");
    let proj = vec![project("hello", &["p", "iy"])];
    let mut overrides = HashMap::new();
    overrides.insert(0usize, SyllableOverride::new("hello", &["m", "ow"]));
    let assigned = resolve_clip_pronunciation(&draft, &[], 1, &overrides, &proj, &[]);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Edited);
    assert_eq!(assigned[0].phonemes, vec!["m", "ow"]);
}

#[test]
fn empty_override_is_inert_and_falls_through() {
    // An override with no phonemes must not blank the syllable — it falls
    // back to the dictionary entry.
    let draft = line("hello");
    let proj = vec![project("hello", &["p", "iy"])];
    let mut overrides = HashMap::new();
    overrides.insert(0usize, SyllableOverride::new::<&str>("hello", &[]));
    let assigned = resolve_clip_pronunciation(&draft, &[], 1, &overrides, &proj, &[]);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Dict);
    assert_eq!(assigned[0].phonemes, vec!["p", "iy"]);
}

// ---------------------------------------------------------------------------
// Override keying: the syllable's index in the resolved draft + its text
// ---------------------------------------------------------------------------

#[test]
fn override_targets_the_right_syllable_among_several() {
    // Two-syllable word over two notes: an override on syllable 1 must
    // touch only the second syllable, leaving the first on the auto path.
    let draft = line("hel·lo");
    let annotations = vec![String::new(), String::new()];
    let mut overrides = HashMap::new();
    overrides.insert(1usize, SyllableOverride::new("lo", &["z", "z"]));
    let assigned = resolve_clip_pronunciation(&draft, &annotations, 2, &overrides, &[], &[]);
    assert_eq!(assigned.len(), 2);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Auto);
    assert_eq!(assigned[1].provenance, PhonemeProvenance::Edited);
    assert_eq!(assigned[1].phonemes, vec!["z", "z"]);
}

#[test]
fn override_follows_its_syllable_past_a_hold() {
    // `go _ now`: three notes, two syllables. The `_` hold takes a note
    // but not a syllable index, so syllable 1 ("now") lands on note 2 and
    // the hold note stays a hold.
    let draft = line("go _ now");
    let mut overrides = HashMap::new();
    overrides.insert(1usize, SyllableOverride::new("now", &["n", "aa"]));
    let assigned = resolve_clip_pronunciation(&draft, &[], 3, &overrides, &[], &[]);
    assert_eq!(assigned.len(), 3);
    assert!(assigned[1].is_slur);
    assert_ne!(assigned[1].provenance, PhonemeProvenance::Edited);
    assert_eq!(assigned[2].provenance, PhonemeProvenance::Edited);
    assert_eq!(assigned[2].phonemes, vec!["n", "aa"]);
}

#[test]
fn override_whose_syllable_text_changed_is_dropped() {
    // The override was made on "hello"; the lyric now reads "yellow" at
    // the same index, so it no longer applies.
    let mut overrides = HashMap::new();
    overrides.insert(0usize, SyllableOverride::new("hello", &["m", "ow"]));
    let assigned = resolve_clip_pronunciation(&line("yellow"), &[], 1, &overrides, &[], &[]);
    assert_ne!(assigned[0].provenance, PhonemeProvenance::Edited);
    assert_ne!(assigned[0].phonemes, vec!["m", "ow"]);

    // Text matching ignores case and surrounding space.
    let assigned = resolve_clip_pronunciation(&line("Hello"), &[], 1, &overrides, &[], &[]);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Edited);
}

#[test]
fn override_past_the_end_of_the_draft_is_dropped() {
    let mut overrides = HashMap::new();
    overrides.insert(3usize, SyllableOverride::new("lo", &["k"]));
    let assigned = resolve_clip_pronunciation(&line("hel·lo"), &[], 2, &overrides, &[], &[]);
    assert!(assigned.iter().all(|a| a.provenance != PhonemeProvenance::Edited));
}

// ---------------------------------------------------------------------------
// Data-model helpers
// ---------------------------------------------------------------------------

#[test]
fn dictionary_entry_cleans_word_and_canonicalizes_phonemes() {
    // Word is lowercased + stripped to letters/apostrophe; bogus phonemes
    // are dropped, valid ones canonicalised.
    let entry = DictionaryEntry::new("Don't!", &["HH", "xx", "ah"], DictionaryScope::Project);
    assert_eq!(entry.word, "don't");
    assert_eq!(entry.phonemes, vec!["hh", "ah"]);
    assert_eq!(entry.scope, DictionaryScope::Project);
}

#[test]
fn clean_word_matches_g2p_lookup_key() {
    assert_eq!(clean_word("  Hello, "), "hello");
    assert_eq!(clean_word("ROCK'N"), "rock'n");
    assert_eq!(clean_word("123"), "");
}

#[test]
fn canonicalize_drops_invalid_keeps_silence_markers() {
    assert_eq!(
        canonicalize_phonemes(&["AH", "AP", "nope", "SP"]),
        vec!["ah", "AP", "SP"]
    );
}

#[test]
fn syllable_override_variant_metadata_is_carried() {
    let ov = SyllableOverride::from_variant("read", vec!["r", "eh", "d"], 2);
    assert_eq!(ov.variant_idx, Some(2));
    assert_eq!(ov.text, "read");
    assert!(ov.is_effective());

    let empty = SyllableOverride::new::<&str>("read", &[]);
    assert_eq!(empty.variant_idx, None);
    assert!(!empty.is_effective());
}

#[test]
fn state_set_remove_and_clear() {
    let (def, track) = (7u64, 2u64);
    let mut state = PronunciationState::default();
    assert!(state.lane_overrides(def, track).is_none());

    state.set_override(def, track, 0, SyllableOverride::new("a", &["ah"]));
    state.set_override(def, track, 1, SyllableOverride::new("e", &["iy"]));
    assert_eq!(state.lane_overrides(def, track).unwrap().len(), 2);

    let removed = state.remove_override(def, track, 0);
    assert_eq!(removed.unwrap().phonemes, vec!["ah"]);
    assert_eq!(state.lane_overrides(def, track).unwrap().len(), 1);

    // Removing the last override drops the lane's sub-map entirely.
    state.remove_override(def, track, 1);
    assert!(state.lane_overrides(def, track).is_none());

    state.project_dictionary.push(project("hello", &["hh"]));
    state.set_override(def, track, 0, SyllableOverride::new("a", &["ah"]));
    state.clear();
    assert!(state.project_dictionary.is_empty());
    assert!(state.lane_overrides(def, track).is_none());
}

#[test]
fn state_resolve_lane_applies_its_own_overrides_and_dictionary() {
    // The method form pulls overrides + project dict from `self` and
    // layers the supplied global dict underneath.
    let draft = line("hello");
    let (def, track) = (3u64, 1u64);
    let mut state = PronunciationState::default();
    state.project_dictionary.push(project("hello", &["p", "iy"]));

    // Dictionary hit via the method.
    let assigned = state.resolve_lane(def, track, &draft, &[], 1, &[]);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Dict);
    assert_eq!(assigned[0].phonemes, vec!["p", "iy"]);

    // Add an override on the lane — now it wins.
    state.set_override(def, track, 0, SyllableOverride::new("hello", &["m", "ow"]));
    let assigned = state.resolve_lane(def, track, &draft, &[], 1, &[]);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Edited);
    assert_eq!(assigned[0].phonemes, vec!["m", "ow"]);

    // Another lane (same section, other track; other section, same
    // track) is unaffected.
    let other = state.resolve_lane(def, 99, &draft, &[], 1, &[]);
    assert_eq!(other[0].provenance, PhonemeProvenance::Dict);
    let other = state.resolve_lane(99, track, &draft, &[], 1, &[]);
    assert_eq!(other[0].provenance, PhonemeProvenance::Dict);
}

// ---------------------------------------------------------------------------
// Persisted form (BUG-50)
// ---------------------------------------------------------------------------

#[test]
fn state_round_trips_through_the_project_form() {
    let mut state = PronunciationState::default();
    state.project_dictionary.push(project("tomato", &["t", "ah", "m", "aa", "t", "ow"]));
    state.project_dictionary.push(project("hello", &["p", "iy"]));
    state.set_override(5, 2, 1, SyllableOverride::new("lo", &["l", "ow"]));
    state.set_override(4, 9, 0, SyllableOverride::from_variant("read", vec!["r", "eh", "d"], 2));

    let file = state.to_project();
    assert_eq!(
        file.dictionary.iter().map(|e| e.word.as_str()).collect::<Vec<_>>(),
        vec!["tomato", "hello"],
        "the dictionary keeps its live order"
    );
    assert_eq!(
        file.overrides
            .iter()
            .map(|o| (o.definition_id, o.track_id, o.syllable_index))
            .collect::<Vec<_>>(),
        vec![(4, 9, 0), (5, 2, 1)],
        "overrides are sorted by lane then syllable"
    );
    assert_eq!(file.overrides[0].variant_idx, Some(2));
    assert_eq!(file.overrides[1].text, "lo");

    let mut restored = PronunciationState::default();
    restored.restore_from_project(&file);
    assert_eq!(restored.to_project(), file);
    assert_eq!(
        restored.lane_overrides(4, 9).unwrap()[&0],
        SyllableOverride::from_variant("read", vec!["r", "eh", "d"], 2)
    );
}

#[test]
fn restore_drops_unknown_phonemes_and_empty_dictionary_entries() {
    let file = ProjectPronunciation {
        dictionary: vec![
            ProjectPronunciationEntry {
                word: "hello".into(),
                phonemes: vec!["p".into(), "bogus".into(), "iy".into()],
            },
            ProjectPronunciationEntry {
                word: "nothing".into(),
                phonemes: vec!["bogus".into()],
            },
        ],
        overrides: vec![ProjectSyllableOverride {
            definition_id: 1,
            track_id: 1,
            syllable_index: 0,
            text: "hello".into(),
            phonemes: vec!["m".into(), "xx".into()],
            variant_idx: None,
        }],
    };
    let mut state = PronunciationState::default();
    state.restore_from_project(&file);
    assert_eq!(state.project_dictionary.len(), 1);
    assert_eq!(state.project_dictionary[0].phonemes, vec!["p", "iy"]);
    assert_eq!(state.lane_overrides(1, 1).unwrap()[&0].phonemes, vec!["m"]);
}
