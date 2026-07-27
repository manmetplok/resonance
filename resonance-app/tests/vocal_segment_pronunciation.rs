//! Tests for wiring the resolved pronunciation (override > project-dict >
//! global-dict > CMU-auto) + voicebank validation into the `.ds` segment
//! build (design #173, notes #174, todo #494).
//!
//! The #493 resolver and the #492 voicebank accessor are tested in their
//! own suites; here we pin the *integration*:
//!
//! * dictionary entries and per-syllable overrides change the phonemes
//!   the segment builder lays into `ph_seq`;
//! * the active voicebank's substitution (Lilia `v` → `f`) is applied
//!   before the phonemes reach the segment;
//! * a phoneme that is not valid ARPAbet (or that the voicebank can't
//!   sing) blocks the render and is reported rather than corrupting the
//!   segment;
//! * provenance survives the validation pass.

use std::collections::HashMap;

use resonance_app::compose::vocal_svs::{
    build_segment, resolve_clip_pronunciation, validate_for_voicebank, DictionaryEntry,
    DictionaryScope, InvalidPhonemeReason, PhonemeProvenance, SyllableOverride,
};
use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::derive::LyricLine;
use resonance_music_theory::{VocalParams, VocalVoicebank};

/// A one-line, one-syllable draft from `text`.
fn line(text: &str) -> Vec<LyricLine> {
    vec![LyricLine {
        n: 1,
        rhyme: 'A',
        syllables: 1,
        text: text.to_string(),
        locked: false,
    }]
}

/// Vocal params on `vb` with a single-line draft of `text`.
fn params(vb: VocalVoicebank, text: &str) -> VocalParams {
    VocalParams {
        voicebank: vb,
        draft: line(text),
        ..VocalParams::default()
    }
}

/// One quarter-note clip so the segment builder has exactly one note to
/// assign the single syllable to.
fn one_note() -> Vec<MidiNote> {
    vec![MidiNote {
        note: 60,
        velocity: 0.8,
        start_tick: 0,
        duration_ticks: TICKS_PER_QUARTER_NOTE,
    }]
}

/// Build a segment from a pre-resolved/validated syllable stream and
/// return its `ph_seq` (silence pads included).
fn ph_seq(vb: VocalVoicebank, text: &str, assigned: &[resonance_music_theory::g2p::AssignedSyllable]) -> Vec<String> {
    build_segment(
        &one_note(),
        &params(vb, text),
        assigned,
        TICKS_PER_QUARTER_NOTE as u32,
        120.0,
    )
    .ph_seq
}

fn contains(seq: &[String], ph: &str) -> bool {
    seq.iter().any(|p| p.as_str() == ph)
}

// ---------------------------------------------------------------------------
// Dictionary application
// ---------------------------------------------------------------------------

#[test]
fn project_dictionary_entry_changes_ph_seq() {
    // A project-dict entry for the word overrides the CMU transcription;
    // the segment builder lays the dictionary phonemes into ph_seq.
    let draft = line("vee");
    let dict = vec![DictionaryEntry::new("vee", &["v", "iy"], DictionaryScope::Project)];
    let assigned = resolve_clip_pronunciation(&draft, &[], 1, &HashMap::new(), &dict, &[]);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Dict);

    let validated = validate_for_voicebank(&assigned, VocalVoicebank::Tiger).unwrap();
    let seq = ph_seq(VocalVoicebank::Tiger, "vee", &validated);
    assert!(contains(&seq, "v"), "dict phoneme `v` missing from {seq:?}");
    assert!(contains(&seq, "iy"), "dict phoneme `iy` missing from {seq:?}");
}

// ---------------------------------------------------------------------------
// Per-syllable override
// ---------------------------------------------------------------------------

#[test]
fn override_changes_ph_seq() {
    let draft = line("go");
    let mut overrides = HashMap::new();
    overrides.insert(0usize, SyllableOverride::new(&["m", "ow"]));
    let assigned = resolve_clip_pronunciation(&draft, &[], 1, &overrides, &[], &[]);
    assert_eq!(assigned[0].provenance, PhonemeProvenance::Edited);

    let validated = validate_for_voicebank(&assigned, VocalVoicebank::Tiger).unwrap();
    let seq = ph_seq(VocalVoicebank::Tiger, "go", &validated);
    assert!(contains(&seq, "m"), "override phoneme `m` missing from {seq:?}");
    assert!(contains(&seq, "ow"), "override phoneme `ow` missing from {seq:?}");
}

// ---------------------------------------------------------------------------
// Voicebank substitution (Lilia v -> f)
// ---------------------------------------------------------------------------

#[test]
fn lilia_substitutes_v_to_f_in_ph_seq() {
    let draft = line("vee");
    let dict = vec![DictionaryEntry::new("vee", &["v", "iy"], DictionaryScope::Project)];
    let assigned = resolve_clip_pronunciation(&draft, &[], 1, &HashMap::new(), &dict, &[]);

    let validated = validate_for_voicebank(&assigned, VocalVoicebank::Lilia).unwrap();
    // The validation pass rewrites the phonemes to their effective form...
    assert_eq!(validated[0].phonemes, vec!["f", "iy"]);
    // ...and provenance is carried through unchanged.
    assert_eq!(validated[0].provenance, PhonemeProvenance::Dict);

    let seq = ph_seq(VocalVoicebank::Lilia, "vee", &validated);
    assert!(contains(&seq, "f"), "expected substituted `f` in {seq:?}");
    assert!(!contains(&seq, "v"), "unsupported `v` leaked into {seq:?}");
}

#[test]
fn tiger_keeps_v_unsubstituted() {
    let draft = line("vee");
    let dict = vec![DictionaryEntry::new("vee", &["v", "iy"], DictionaryScope::Project)];
    let assigned = resolve_clip_pronunciation(&draft, &[], 1, &HashMap::new(), &dict, &[]);

    let validated = validate_for_voicebank(&assigned, VocalVoicebank::Tiger).unwrap();
    assert_eq!(validated[0].phonemes, vec!["v", "iy"]);
    let seq = ph_seq(VocalVoicebank::Tiger, "vee", &validated);
    assert!(contains(&seq, "v"), "TIGER should sing `v` directly: {seq:?}");
}

// ---------------------------------------------------------------------------
// Invalid-phoneme block
// ---------------------------------------------------------------------------

#[test]
fn non_arpabet_phoneme_blocks_render() {
    // Inject a garbage symbol that bypassed canonicalisation. The
    // voicebank accessor's denylist would wave it through, so the ARPAbet
    // gate (#491) is the safety net that keeps it off the model.
    let mut assigned = resolve_clip_pronunciation(&line("hello"), &[], 1, &HashMap::new(), &[], &[]);
    assigned[0].phonemes = vec!["zzz"];

    let err = validate_for_voicebank(&assigned, VocalVoicebank::Tiger).unwrap_err();
    assert_eq!(err.len(), 1);
    assert_eq!(err[0].reason, InvalidPhonemeReason::NotArpabet);
    assert_eq!(err[0].phoneme, "zzz");
    assert_eq!(err[0].note_index, 0);
}

#[test]
fn all_invalid_phonemes_are_reported_with_note_indices() {
    // Two notes, both with a bad phoneme: every offender is collected so
    // the UI can badge each affected note, not just the first.
    let mut assigned =
        resolve_clip_pronunciation(&line("go go"), &[], 2, &HashMap::new(), &[], &[]);
    assert_eq!(assigned.len(), 2);
    assigned[0].phonemes = vec!["qq"];
    assigned[1].phonemes = vec!["xx"];

    let err = validate_for_voicebank(&assigned, VocalVoicebank::Tiger).unwrap_err();
    assert_eq!(err.len(), 2);
    assert_eq!(err[0].note_index, 0);
    assert_eq!(err[1].note_index, 1);
}

#[test]
fn valid_resolution_is_not_blocked() {
    // The ordinary auto path clears both gates on every shipped bank.
    let assigned = resolve_clip_pronunciation(&line("hello"), &[], 1, &HashMap::new(), &[], &[]);
    for vb in [
        VocalVoicebank::Tiger,
        VocalVoicebank::Lilia,
        VocalVoicebank::Meiji,
    ] {
        assert!(
            validate_for_voicebank(&assigned, vb).is_ok(),
            "{vb:?} should sing the auto transcription of `hello`"
        );
    }
}
