//! A finished recording is an undoable, dirtying edit (STATE-02).
//!
//! Recording starts and stops through `Skip` transport messages; the take
//! lands later, through an engine event (`RecordingFinished`, a
//! cycle-record `TakeCaptured`, or the `MidiClipCreated` a live MIDI
//! recording opens). Before the fix none of those set `dirty` or touched
//! the history, so closing the window lost the take without a prompt and
//! undoing an unrelated earlier edit dropped the take through the
//! full-reload path. Each recording session is now one history entry
//! holding the pre-take state, recorded like any other edit: it marks the
//! project dirty, bumps the control revision and clears the redo stack.

use resonance_app::message::{Message, TrackMessage};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, MidiNote, TrackType};
use resonance_common::{TakeContent, TimelineRange};

const TRACK: u64 = 1;
const CLIP: u64 = 7;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    // The history only records once the project has a path on disk.
    app.test_set_project_path(std::path::PathBuf::from(
        "/tmp/timeline-recording-undo.rprj",
    ));
    app.test_add_track(TRACK, TrackType::Audio);
    app
}

fn finished(clip_id: u64) -> AudioEvent {
    AudioEvent::RecordingFinished {
        clip_id,
        track_id: TRACK,
        start_sample: 0,
        duration_samples: 48_000,
        name: format!("Recording {clip_id}"),
        waveform_peaks: Vec::new(),
    }
}

fn volume(app: &Resonance) -> f32 {
    app.test_registry()
        .tracks
        .iter()
        .find(|t| t.id == TRACK)
        .expect("test track")
        .volume
}

fn entries(app: &Resonance) -> usize {
    app.test_undo_history().test_undo_entries().len()
}

#[test]
fn a_finished_recording_marks_dirty_bumps_revision_and_records_one_entry() {
    let mut app = app();
    app.test_set_dirty(false);
    let revision = app.revision();

    app.test_apply_engine_event(finished(CLIP));

    assert!(app.is_dirty(), "a recorded take must mark the project dirty");
    assert_eq!(app.revision(), revision + 1, "one revision bump per take");
    assert_eq!(entries(&app), 1, "the take is one undo entry");
    assert!(app.test_clips().iter().any(|c| c.id == CLIP));
}

#[test]
fn undoing_an_earlier_edit_first_undoes_only_the_take() {
    let mut app = app();
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -6.0)));
    app.test_apply_engine_event(finished(CLIP));
    assert_eq!(entries(&app), 2);

    // The first undo backs out the take (a structural change → the slow
    // path, which completes on the engine's `AllCleared`) and nothing else.
    let _ = app.update(Message::Undo);
    app.test_apply_engine_event(AudioEvent::AllCleared);
    assert!(
        !app.test_clips().iter().any(|c| c.id == CLIP),
        "the first undo removes the take"
    );
    assert!(
        (volume(&app) - -6.0).abs() < 1e-4,
        "the fader move survives the undo of the take"
    );
    assert!(app.test_undo_history().can_redo(), "the take is redoable");
}

#[test]
fn a_finished_recording_clears_the_redo_stack() {
    let mut app = app();
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -6.0)));
    let _ = app.update(Message::Undo);
    assert!(app.test_undo_history().can_redo());

    app.test_apply_engine_event(finished(CLIP));
    assert!(
        !app.test_undo_history().can_redo(),
        "a take is a new edit: redo snapshots must not outlive it (STATE-08)"
    );
}

#[test]
fn every_track_of_one_recording_session_lands_in_one_entry() {
    let mut app = app();
    app.test_apply_engine_event(AudioEvent::RecordingStarted { start_sample: 0 });
    app.test_apply_engine_event(finished(CLIP));
    app.test_apply_engine_event(finished(CLIP + 1));
    assert_eq!(entries(&app), 1, "one stop of the transport, one entry");

    // A second recording session is its own entry.
    app.test_apply_engine_event(AudioEvent::RecordingStarted { start_sample: 0 });
    app.test_apply_engine_event(finished(CLIP + 2));
    assert_eq!(entries(&app), 2);
}

#[test]
fn a_cycle_record_take_is_undoable_and_dirty() {
    let mut app = app();
    app.test_set_dirty(false);
    let revision = app.revision();
    let slot = TimelineRange {
        start: 0,
        length: 48_000,
    };
    app.test_apply_engine_event(AudioEvent::RecordingStarted { start_sample: 0 });
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: 1,
        take_id: 0,
        track_id: TRACK,
        slot,
        pass_index: 0,
        extent: slot,
        content: TakeContent::Audio { clip_ref: 5_000 },
    });
    assert!(app.is_dirty());
    assert_eq!(app.revision(), revision + 1);
    assert_eq!(entries(&app), 1);
    assert!(
        app.test_undo_history().test_undo_entries()[0]
            .project
            .file
            .take_groups
            .is_empty(),
        "the entry holds the pre-take state"
    );
}

#[test]
fn a_live_midi_recording_is_undoable_and_dirty() {
    let mut app = app();
    app.test_set_dirty(false);
    app.test_apply_engine_event(AudioEvent::RecordingStarted { start_sample: 0 });
    let revision = app.revision();
    app.test_apply_engine_event(AudioEvent::MidiClipCreated {
        clip_id: CLIP,
        track_id: TRACK,
        start_sample: 0,
        duration_ticks: 0,
        name: format!("MIDI Take {CLIP}"),
        notes: Vec::new(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    app.test_apply_engine_event(AudioEvent::MidiNoteAdded {
        clip_id: CLIP,
        note: MidiNote {
            note: 60,
            velocity: 1.0,
            start_tick: 0,
            duration_ticks: 0,
        },
    });
    assert!(app.is_dirty());
    assert_eq!(app.revision(), revision + 1);
    assert_eq!(entries(&app), 1, "the whole MIDI take is one entry");
    assert!(
        app.test_undo_history().test_undo_entries()[0]
            .project
            .file
            .midi_clips
            .is_empty(),
        "the entry holds the pre-take state"
    );
}

#[test]
fn a_midi_clip_created_outside_recording_is_not_recorded_again() {
    // A `MidiClipCreated` echo of an app-issued edit is already covered by
    // that message's own history entry.
    let mut app = app();
    app.test_apply_engine_event(AudioEvent::MidiClipCreated {
        clip_id: CLIP,
        track_id: TRACK,
        start_sample: 0,
        duration_ticks: 0,
        name: "clip".into(),
        notes: Vec::new(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    assert_eq!(entries(&app), 0);
}

/// A take that lands while a drag gesture is open (FU-A2a). The gesture's
/// pre-drag snapshot used to be committed *after* the take's entry, so
/// undoing the drag restored a state that predates the take — the take
/// vanished with it. The take now splits the gesture: the part of the
/// drag before it, the take, and the rest of the drag are three entries,
/// undone newest first.
#[test]
fn a_take_landing_mid_drag_survives_undoing_the_drag() {
    use resonance_app::message::TransportMessage;
    use resonance_app::state::LoopDragTarget;

    let mut app = app();
    let (_, loop_out_before, _) = app.test_loop_range();
    let _ = app.update(Message::Transport(TransportMessage::StartLoopDrag(
        LoopDragTarget::Out,
    )));
    let _ = app.update(Message::Transport(TransportMessage::UpdateLoopDrag(200.0)));
    let (_, loop_out_mid, _) = app.test_loop_range();
    app.test_apply_engine_event(finished(CLIP));
    let _ = app.update(Message::Transport(TransportMessage::UpdateLoopDrag(800.0)));
    let _ = app.update(Message::Transport(TransportMessage::EndLoopDrag));
    let (_, loop_out_end, _) = app.test_loop_range();
    assert!(
        loop_out_before != loop_out_mid && loop_out_mid != loop_out_end,
        "each half of the drag moves the loop end, or this test is vacuous"
    );

    // Undo the rest of the drag: the take stays.
    let _ = app.update(Message::Undo);
    app.test_apply_engine_event(AudioEvent::AllCleared);
    assert!(
        app.test_clips().iter().any(|c| c.id == CLIP),
        "undoing the drag keeps the take that landed during it"
    );
    assert_eq!(app.test_loop_range().1, loop_out_mid);

    // Then the take, then the part of the drag before it.
    let _ = app.update(Message::Undo);
    app.test_apply_engine_event(AudioEvent::AllCleared);
    assert!(!app.test_clips().iter().any(|c| c.id == CLIP));
    assert_eq!(app.test_loop_range().1, loop_out_mid);
    let _ = app.update(Message::Undo);
    app.test_apply_engine_event(AudioEvent::AllCleared);
    assert_eq!(app.test_loop_range().1, loop_out_before);
    assert!(!app.test_undo_history().can_undo());
}
