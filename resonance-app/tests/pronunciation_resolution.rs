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
use resonance_audio::types::ClipId;
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
    overrides.insert(0usize, SyllableOverride::new(&["m", "ow"]));
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
    overrides.insert(0usize, SyllableOverride::new::<&str>(&[]));
    let assigned = resolve_clip_pronunciation(&draft, &[], 1, &overrides, &proj, &[]);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Dict);
    assert_eq!(assigned[0].phonemes, vec!["p", "iy"]);
}

// ---------------------------------------------------------------------------
// Per-note override keying (the app keys by note; g2p keys by syllable)
// ---------------------------------------------------------------------------

#[test]
fn override_targets_the_right_note_among_several() {
    // Two-syllable word over two notes: an override on note 1 must touch
    // only the second syllable, leaving the first on the auto path.
    let draft = line("hel·lo");
    let annotations = vec![String::new(), String::new()];
    let mut overrides = HashMap::new();
    overrides.insert(1usize, SyllableOverride::new(&["z", "z"]));
    let assigned = resolve_clip_pronunciation(&draft, &annotations, 2, &overrides, &[], &[]);
    assert_eq!(assigned.len(), 2);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Auto);
    assert_eq!(assigned[1].provenance, PhonemeProvenance::Edited);
    assert_eq!(assigned[1].phonemes, vec!["z", "z"]);
}

#[test]
fn override_pinned_to_a_slur_note_is_dropped() {
    // Note 1 is a slur holding note 0's vowel; slur notes carry no
    // override of their own, so an override keyed to the slur is ignored
    // and the note stays a slur.
    let draft = line("hello");
    let annotations = vec![String::new(), "+".to_string()];
    let mut overrides = HashMap::new();
    overrides.insert(1usize, SyllableOverride::new(&["k"]));
    let assigned = resolve_clip_pronunciation(&draft, &annotations, 2, &overrides, &[], &[]);
    assert_eq!(assigned.len(), 2);
    assert!(assigned[1].is_slur);
    assert_ne!(
        assigned[1].provenance,
        PhonemeProvenance::Edited,
        "a slur note must not pick up an override"
    );
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
    let ov = SyllableOverride::from_variant(vec!["r", "eh", "d"], 2);
    assert_eq!(ov.variant_idx, Some(2));
    assert!(ov.is_effective());

    let empty = SyllableOverride::new::<&str>(&[]);
    assert_eq!(empty.variant_idx, None);
    assert!(!empty.is_effective());
}

#[test]
fn state_set_remove_and_clear() {
    let clip: ClipId = 7;
    let mut state = PronunciationState::default();
    assert!(state.clip_overrides(clip).is_none());

    state.set_override(clip, 0, SyllableOverride::new(&["ah"]));
    state.set_override(clip, 1, SyllableOverride::new(&["iy"]));
    assert_eq!(state.clip_overrides(clip).unwrap().len(), 2);

    let removed = state.remove_override(clip, 0);
    assert_eq!(removed.unwrap().phonemes, vec!["ah"]);
    assert_eq!(state.clip_overrides(clip).unwrap().len(), 1);

    // Removing the last override drops the clip's sub-map entirely.
    state.remove_override(clip, 1);
    assert!(state.clip_overrides(clip).is_none());

    state.project_dictionary.push(project("hello", &["hh"]));
    state.set_override(clip, 0, SyllableOverride::new(&["ah"]));
    state.clear();
    assert!(state.project_dictionary.is_empty());
    assert!(state.clip_overrides(clip).is_none());
}

#[test]
fn state_resolve_clip_applies_its_own_overrides_and_dictionary() {
    // The method form pulls overrides + project dict from `self` and
    // layers the supplied global dict underneath.
    let draft = line("hello");
    let clip: ClipId = 3;
    let mut state = PronunciationState::default();
    state.project_dictionary.push(project("hello", &["p", "iy"]));

    // Dictionary hit via the method.
    let assigned = state.resolve_clip(clip, &draft, &[], 1, &[]);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Dict);
    assert_eq!(assigned[0].phonemes, vec!["p", "iy"]);

    // Add an override on the clip — now it wins.
    state.set_override(clip, 0, SyllableOverride::new(&["m", "ow"]));
    let assigned = state.resolve_clip(clip, &draft, &[], 1, &[]);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Edited);
    assert_eq!(assigned[0].phonemes, vec!["m", "ow"]);

    // A different clip is unaffected by clip 3's override.
    let other = state.resolve_clip(99, &draft, &[], 1, &[]);
    assert_eq!(other[0].provenance, PhonemeProvenance::Dict);
}
