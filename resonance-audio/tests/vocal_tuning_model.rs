//! Unit tests for the non-destructive vocal-tuning data model on
//! `AudioClip` (todo #356). Covers the zero-overhead default, the edit
//! mutation helpers, clamping/saturation, and that attaching/editing
//! tuning never touches the clip's PCM source.

use resonance_audio::types::{
    AudioClip, ClipSource, FadeCurve, GlobalTuning, NoteBlob, NoteEdit, TuningScale, VocalTuning,
};

fn make_clip(frames: usize) -> AudioClip {
    AudioClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        // Distinct non-zero samples so we can assert the PCM is untouched.
        source: ClipSource::Memory((0..frames * 2).map(|i| i as f32 * 0.001).collect()),
        name: "vox".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        vocal_tuning: None,
    }
}

fn make_note(start: u64, end: u64, midi: f32) -> NoteBlob {
    NoteBlob {
        start_frame: start,
        end_frame: end,
        mean_pitch_midi: midi,
        cents_contour: vec![0.0, 5.0, -3.0],
        edit: NoteEdit::default(),
    }
}

#[test]
fn clip_default_is_untuned_zero_overhead() {
    let clip = make_clip(64);
    assert!(!clip.is_tuned());
    assert!(clip.vocal_tuning.is_none());
}

#[test]
fn vocal_tuning_default_is_empty_and_natural() {
    let vt = VocalTuning::default();
    assert!(vt.contour.is_empty());
    assert!(vt.notes.is_empty());
    assert_eq!(vt.global, GlobalTuning::default());
    assert_eq!(vt.global.scale, TuningScale::Chromatic);
    assert_eq!(vt.global.correction_amount, 0.0);
    assert!(!vt.has_edits());
}

#[test]
fn note_edit_default_is_identity() {
    let e = NoteEdit::default();
    assert_eq!(e.semitone_offset, 0.0);
    assert_eq!(e.correction_strength, 0.0);
    assert_eq!(e.drift, 1.0);
    assert_eq!(e.timing_nudge_frames, 0);
    assert!(e.is_identity());
}

#[test]
fn vocal_tuning_mut_inserts_default_on_first_use() {
    let mut clip = make_clip(64);
    assert!(!clip.is_tuned());
    let vt = clip.vocal_tuning_mut();
    vt.notes.push(make_note(0, 100, 60.0));
    assert!(clip.is_tuned());
    assert_eq!(clip.vocal_tuning.as_ref().unwrap().notes.len(), 1);
}

#[test]
fn editing_tuning_never_mutates_pcm() {
    let mut clip = make_clip(32);
    let before: Vec<f32> = clip.source.as_frames().to_vec();

    let vt = clip.vocal_tuning_mut();
    vt.notes.push(make_note(0, 200, 57.0));
    vt.set_note_semitone_offset(0, 2.0);
    vt.set_note_correction_strength(0, 1.0);
    vt.set_key_scale(7, TuningScale::Major);

    let after: Vec<f32> = clip.source.as_frames().to_vec();
    assert_eq!(before, after, "PCM source must be untouched by tuning edits");
}

#[test]
fn set_note_semitone_offset_updates_target_pitch() {
    let mut vt = VocalTuning::default();
    vt.notes.push(make_note(0, 100, 60.0));
    assert!(vt.set_note_semitone_offset(0, 3.0));
    let note = &vt.notes[0];
    assert_eq!(note.edit.semitone_offset, 3.0);
    assert_eq!(note.target_pitch_midi(), 63.0);
    assert!(vt.has_edits() == false, "offset alone with 0 strength is identity");
    // Out-of-range index is a no-op.
    assert!(!vt.set_note_semitone_offset(5, 1.0));
}

#[test]
fn correction_strength_and_drift_are_clamped() {
    let mut vt = VocalTuning::default();
    vt.notes.push(make_note(0, 100, 60.0));

    assert!(vt.set_note_correction_strength(0, 2.5));
    assert_eq!(vt.notes[0].edit.correction_strength, 1.0);
    assert!(vt.set_note_correction_strength(0, -1.0));
    assert_eq!(vt.notes[0].edit.correction_strength, 0.0);

    assert!(vt.set_note_drift(0, 9.0));
    assert_eq!(vt.notes[0].edit.drift, 1.0);
    assert!(vt.set_note_drift(0, -4.0));
    assert_eq!(vt.notes[0].edit.drift, 0.0);
}

#[test]
fn correction_amount_is_clamped_and_counts_as_edit() {
    let mut vt = VocalTuning::default();
    vt.notes.push(make_note(0, 100, 60.0));
    vt.set_correction_amount(3.0);
    assert_eq!(vt.global.correction_amount, 1.0);
    assert!(vt.has_edits());
    vt.set_correction_amount(-1.0);
    assert_eq!(vt.global.correction_amount, 0.0);
    assert!(!vt.has_edits());
}

#[test]
fn nudge_note_timing_accumulates_and_saturates() {
    let mut vt = VocalTuning::default();
    vt.notes.push(make_note(0, 100, 60.0));

    assert!(vt.nudge_note_timing(0, 50));
    assert!(vt.nudge_note_timing(0, -20));
    assert_eq!(vt.notes[0].edit.timing_nudge_frames, 30);
    assert!(vt.has_edits());

    // Saturates rather than overflowing.
    vt.notes[0].edit.timing_nudge_frames = i64::MAX - 5;
    assert!(vt.nudge_note_timing(0, 100));
    assert_eq!(vt.notes[0].edit.timing_nudge_frames, i64::MAX);

    assert!(!vt.nudge_note_timing(9, 1));
}

#[test]
fn set_key_scale_wraps_key_to_pitch_class() {
    let mut vt = VocalTuning::default();
    vt.set_key_scale(14, TuningScale::Minor);
    assert_eq!(vt.global.key, 2);
    assert_eq!(vt.global.scale, TuningScale::Minor);
}

#[test]
fn reset_edits_keeps_analysis_and_key_but_clears_edits() {
    let mut vt = VocalTuning::default();
    vt.notes.push(make_note(0, 100, 60.0));
    vt.notes.push(make_note(120, 240, 62.0));
    vt.set_key_scale(5, TuningScale::Dorian);
    vt.set_correction_amount(0.8);
    vt.set_note_semitone_offset(0, 2.0);
    vt.set_note_correction_strength(0, 0.9);
    vt.nudge_note_timing(1, 40);
    assert!(vt.has_edits());

    vt.reset_edits();

    // Detection geometry and key/scale survive.
    assert_eq!(vt.notes.len(), 2);
    assert_eq!(vt.notes[0].mean_pitch_midi, 60.0);
    assert_eq!(vt.global.key, 5);
    assert_eq!(vt.global.scale, TuningScale::Dorian);
    // Edits are gone.
    assert_eq!(vt.global.correction_amount, 0.0);
    assert_eq!(vt.notes[0].edit, NoteEdit::default());
    assert_eq!(vt.notes[1].edit, NoteEdit::default());
    assert!(!vt.has_edits());
}

#[test]
fn note_blob_geometry_helpers() {
    let note = make_note(480, 1440, 69.0);
    assert_eq!(note.duration_frames(), 960);
    assert_eq!(note.target_pitch_midi(), 69.0);
}

#[test]
fn tuning_scale_intervals_match_modes() {
    assert_eq!(TuningScale::Chromatic.intervals(), None);
    assert_eq!(TuningScale::Major.intervals(), Some(&[0, 2, 4, 5, 7, 9, 11][..]));
    assert_eq!(TuningScale::Minor.intervals(), Some(&[0, 2, 3, 5, 7, 8, 10][..]));
    assert_eq!(
        TuningScale::HarmonicMinor.intervals(),
        Some(&[0, 2, 3, 5, 7, 8, 11][..])
    );
}

#[test]
fn vocal_tuning_is_clone_and_debug() {
    let mut vt = VocalTuning::default();
    vt.notes.push(make_note(0, 100, 60.0));
    vt.contour.push(resonance_audio::types::F0Frame {
        frame: 0,
        f0_hz: 261.6,
        confidence: 0.9,
        voiced: true,
    });
    let cloned = vt.clone();
    assert_eq!(vt, cloned);
    let dbg = format!("{vt:?}");
    assert!(dbg.contains("VocalTuning"));
}
