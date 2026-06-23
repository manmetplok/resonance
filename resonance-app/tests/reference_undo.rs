//! Undo/redo coverage for the reference-track (A/B) feature: the
//! classifier picks the right action per message, and the content-
//! changing actions round-trip through the undo history (captured in
//! `UndoExtras` and restored on the fast diff-replay path).

use std::path::PathBuf;

use resonance_app::message::Message;
use resonance_app::reference::ReferenceMessage;
use resonance_app::undo::{classify, CoalesceKey, UndoAction};
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, ReferenceId};

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(PathBuf::from("/tmp/reference-undo.rsn"));
    app
}

fn send(app: &mut Resonance, m: ReferenceMessage) {
    let _ = app.update(Message::Reference(m));
}

fn fold_loaded(app: &mut Resonance, id: u32) {
    app.test_handle_engine_event(AudioEvent::ReferenceLoaded {
        id: ReferenceId(id),
        name: format!("ref{id}"),
        path: format!("/refs/{id}.wav"),
        integrated_lufs: -14.0,
        waveform_peaks: vec![],
        length_samples: 480_000,
    });
}

#[test]
fn classify_marks_content_actions_recordable() {
    use ReferenceMessage as M;
    let record = |m: M| matches!(classify(&Message::Reference(m)), UndoAction::Record);

    assert!(record(M::LoadRequested(PathBuf::from("/a.wav"))));
    assert!(record(M::Remove(ReferenceId(1))));
    assert!(record(M::SetActive(ReferenceId(1))));
    assert!(record(M::ToggleLoudnessMatch));
}

#[test]
fn classify_coalesces_trim() {
    let action = classify(&Message::Reference(ReferenceMessage::TrimChanged(-2.0)));
    assert!(matches!(
        action,
        UndoAction::RecordCoalesced(CoalesceKey::ReferenceTrim)
    ));
}

#[test]
fn classify_skips_transient_actions() {
    use ReferenceMessage as M;
    let skip = |m: M| matches!(classify(&Message::Reference(m)), UndoAction::Skip);

    assert!(skip(M::ToggleAbSource));
    assert!(skip(M::MomentaryAudition(true)));
    assert!(skip(M::ToggleLoopToMix));
    assert!(skip(M::DismissError));
    assert!(skip(M::Scrub {
        ref_id: ReferenceId(1),
        position_samples: 0
    }));
    assert!(skip(M::AddMarker {
        ref_id: ReferenceId(1),
        position_samples: 0,
        label: String::new()
    }));
    assert!(skip(M::RemoveMarker {
        ref_id: ReferenceId(1),
        marker_id: 0
    }));
}

#[test]
fn trim_round_trips_through_undo_redo() {
    let mut app = app();
    fold_loaded(&mut app, 1);

    send(&mut app, ReferenceMessage::TrimChanged(-6.0));
    assert_eq!(app.test_reference().trim_db, -6.0);

    let _ = app.update(Message::Undo);
    assert_eq!(app.test_reference().trim_db, 0.0, "undo restores pre-trim");
    // The loaded entry survives the undo (captured in the snapshot).
    assert_eq!(app.test_reference().entries.len(), 1);

    let _ = app.update(Message::Redo);
    assert_eq!(app.test_reference().trim_db, -6.0, "redo reapplies trim");
}

#[test]
fn loudness_match_round_trips_through_undo() {
    let mut app = app();
    fold_loaded(&mut app, 1);

    send(&mut app, ReferenceMessage::ToggleLoudnessMatch);
    assert!(app.test_reference().loudness_match);

    let _ = app.update(Message::Undo);
    assert!(!app.test_reference().loudness_match);
}

#[test]
fn remove_round_trips_through_undo() {
    let mut app = app();
    fold_loaded(&mut app, 1);
    assert_eq!(app.test_reference().entries.len(), 1);

    send(&mut app, ReferenceMessage::Remove(ReferenceId(1)));
    assert!(app.test_reference().entries.is_empty());

    let _ = app.update(Message::Undo);
    assert_eq!(app.test_reference().entries.len(), 1, "removed ref comes back");
    assert_eq!(app.test_reference().entries[0].id, ReferenceId(1));
}

#[test]
fn undo_of_load_removes_the_reference() {
    let mut app = app();
    // The load action snapshots the pre-load (empty) state. The entry
    // then arrives via the engine echo; undoing the load drops it.
    send(
        &mut app,
        ReferenceMessage::LoadRequested(PathBuf::from("/refs/1.wav")),
    );
    fold_loaded(&mut app, 1);
    assert_eq!(app.test_reference().entries.len(), 1);

    let _ = app.update(Message::Undo);
    assert!(
        app.test_reference().entries.is_empty(),
        "undo of load removes the reference"
    );
}

#[test]
fn transient_toggle_is_not_undoable() {
    let mut app = app();
    send(&mut app, ReferenceMessage::TrimChanged(-3.0));
    // A transient toggle in between must not push a history entry, so the
    // undo still targets the trim.
    send(&mut app, ReferenceMessage::ToggleLoopToMix);
    let _ = app.update(Message::Undo);
    assert_eq!(app.test_reference().trim_db, 0.0);
    // loop_to_mix is live state, untouched by the undo.
    assert!(app.test_reference().loop_to_mix);
}
