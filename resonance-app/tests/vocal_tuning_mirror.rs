//! App-side vocal-tuning state plumbing (todo #359, doc #160).
//!
//! Two halves are exercised here:
//!
//! * **Event mirroring** — receiving `AudioEvent::ClipPitchDetected` folds
//!   the detected f0 `contour` and `notes` into the matching GUI-side
//!   `ClipState::vocal_tuning`, leaving the app-side global key/scale/
//!   correction parameters (which analysis never derives) intact, and
//!   never touching unrelated clips.
//! * **Editor open/close** — `VocalTuningMessage::OpenPitchEditor` records
//!   the open clip only for a *vocal-track* clip (and requests analysis);
//!   it is a no-op for a non-vocal clip or an unknown id, and
//!   `ClosePitchEditor` clears the state.

use resonance_app::message::{Message, VocalTuningMessage};
use resonance_app::state::{ClipState, TrackState};
use resonance_app::Resonance;
use resonance_audio::types::{
    AudioEvent, F0Frame, GlobalTuning, NoteBlob, NoteEdit, TuningScale, VocalTuning,
};

/// A bare audio clip with no vocal-tuning analysis yet.
fn clip(id: u64, track_id: u64) -> ClipState {
    ClipState {
        id,
        track_id,
        start_sample: 1_000,
        duration_samples: 48_000,
        name: format!("clip {id}"),
        total_frames: 48_000,
        trim_start_frames: 0,
        trim_end_frames: 0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
    }
}

fn note(start_frame: u64, end_frame: u64, mean_pitch_midi: f32) -> NoteBlob {
    NoteBlob {
        start_frame,
        end_frame,
        mean_pitch_midi,
        cents_contour: Vec::new(),
        edit: NoteEdit::default(),
    }
}

fn frame(frame: u64, f0_hz: f32) -> F0Frame {
    F0Frame {
        frame,
        f0_hz,
        confidence: 0.9,
        voiced: f0_hz > 0.0,
    }
}

#[test]
fn pitch_detected_populates_vocal_tuning() {
    let mut app = Resonance::new().0;
    app.test_push_clip(clip(7, 1));

    app.test_apply_engine_event(AudioEvent::ClipPitchDetected {
        clip_id: 7,
        notes: vec![note(0, 24_000, 60.0), note(24_000, 48_000, 62.0)],
        contour: vec![frame(0, 261.6), frame(512, 293.7)],
    });

    let tuning = app.test_clips()[0]
        .vocal_tuning
        .as_ref()
        .expect("analysis should have populated the mirror");
    assert_eq!(tuning.notes.len(), 2);
    assert_eq!(tuning.notes[0].mean_pitch_midi, 60.0);
    assert_eq!(tuning.notes[1].mean_pitch_midi, 62.0);
    assert_eq!(tuning.contour.len(), 2);
    assert_eq!(tuning.contour[1].f0_hz, 293.7);
}

#[test]
fn empty_detection_still_initialises_the_mirror() {
    let mut app = Resonance::new().0;
    app.test_push_clip(clip(7, 1));

    // A clip with no voiced material yields empty vectors — the mirror is
    // still created (Some), distinguishing "analysed, nothing found" from
    // "never analysed" (None).
    app.test_apply_engine_event(AudioEvent::ClipPitchDetected {
        clip_id: 7,
        notes: Vec::new(),
        contour: Vec::new(),
    });

    let tuning = app.test_clips()[0]
        .vocal_tuning
        .as_ref()
        .expect("an analysed clip carries Some, even when empty");
    assert!(tuning.notes.is_empty());
    assert!(tuning.contour.is_empty());
}

#[test]
fn re_analysis_replaces_geometry_but_preserves_global_params() {
    let mut app = Resonance::new().0;
    let mut c = clip(7, 1);
    // A prior analysis plus user-set global key/scale/correction.
    c.vocal_tuning = Some(VocalTuning {
        contour: vec![frame(0, 220.0)],
        notes: vec![note(0, 1_000, 57.0)],
        global: GlobalTuning {
            key: 5,
            scale: TuningScale::Minor,
            correction_amount: 0.75,
        },
    });
    app.test_push_clip(c);

    app.test_apply_engine_event(AudioEvent::ClipPitchDetected {
        clip_id: 7,
        notes: vec![note(0, 48_000, 69.0)],
        contour: vec![frame(0, 440.0), frame(256, 440.0)],
    });

    let tuning = app.test_clips()[0].vocal_tuning.as_ref().unwrap();
    // Geometry replaced by the fresh detection.
    assert_eq!(tuning.notes.len(), 1);
    assert_eq!(tuning.notes[0].mean_pitch_midi, 69.0);
    assert_eq!(tuning.contour.len(), 2);
    // Global params (app-side user settings) preserved across re-analysis.
    assert_eq!(tuning.global.key, 5);
    assert_eq!(tuning.global.scale, TuningScale::Minor);
    assert_eq!(tuning.global.correction_amount, 0.75);
}

#[test]
fn pitch_detected_only_touches_the_matching_clip() {
    let mut app = Resonance::new().0;
    app.test_push_clip(clip(7, 1));
    app.test_push_clip(clip(8, 1));

    app.test_apply_engine_event(AudioEvent::ClipPitchDetected {
        clip_id: 8,
        notes: vec![note(0, 100, 64.0)],
        contour: vec![frame(0, 329.6)],
    });

    let clips = app.test_clips();
    let c7 = clips.iter().find(|c| c.id == 7).unwrap();
    let c8 = clips.iter().find(|c| c.id == 8).unwrap();
    assert!(c7.vocal_tuning.is_none(), "unrelated clip stays untouched");
    assert!(c8.vocal_tuning.is_some(), "targeted clip is populated");
}

#[test]
fn pitch_detected_for_unknown_clip_is_a_no_op() {
    let mut app = Resonance::new().0;
    app.test_push_clip(clip(7, 1));

    // No clip 99 — must not panic or mutate the existing clip.
    app.test_apply_engine_event(AudioEvent::ClipPitchDetected {
        clip_id: 99,
        notes: vec![note(0, 100, 60.0)],
        contour: vec![frame(0, 261.6)],
    });

    let c = &app.test_clips()[0];
    assert_eq!(c.id, 7);
    assert!(c.vocal_tuning.is_none());
}

#[test]
fn opening_editor_on_vocal_clip_records_it() {
    let mut app = Resonance::new().0;
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_vocal(1, 0));
    app.test_push_clip(clip(7, 1));

    let _ = app.update(Message::VocalTuning(VocalTuningMessage::OpenPitchEditor(7)));

    assert_eq!(app.test_editing_pitch_clip(), Some(7));
}

#[test]
fn opening_editor_on_non_vocal_clip_is_a_no_op() {
    let mut app = Resonance::new().0;
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_audio(1, 0));
    app.test_push_clip(clip(7, 1));

    let _ = app.update(Message::VocalTuning(VocalTuningMessage::OpenPitchEditor(7)));

    assert_eq!(
        app.test_editing_pitch_clip(),
        None,
        "pitch editing is vocal-only"
    );
}

#[test]
fn opening_editor_on_unknown_clip_is_a_no_op() {
    let mut app = Resonance::new().0;
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_vocal(1, 0));

    // No clip 7 exists.
    let _ = app.update(Message::VocalTuning(VocalTuningMessage::OpenPitchEditor(7)));

    assert_eq!(app.test_editing_pitch_clip(), None);
}

#[test]
fn closing_editor_clears_the_open_clip() {
    let mut app = Resonance::new().0;
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_vocal(1, 0));
    app.test_push_clip(clip(7, 1));

    let _ = app.update(Message::VocalTuning(VocalTuningMessage::OpenPitchEditor(7)));
    assert_eq!(app.test_editing_pitch_clip(), Some(7));

    let _ = app.update(Message::VocalTuning(VocalTuningMessage::ClosePitchEditor));
    assert_eq!(app.test_editing_pitch_clip(), None);
}
