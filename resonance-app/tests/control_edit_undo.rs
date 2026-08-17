//! `edit.undo` / `edit.redo` / `edit.status` over the control endpoint
//! (ba doc #273, todo #1196).
//!
//! Control edits were already undoable — every mutating handler routes
//! through the full update path, which runs `record_undo` — but nothing
//! could pop one back off, so a mis-scoped edit stranded the caller with
//! no recovery path. These methods drive the app's EXISTING history, not
//! a parallel one, which is why the labels matter: the stack is shared
//! with the user's own GUI edits.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::TrackType;
use resonance_control::methods::edit::{EditStatus, RedoResult, UndoResult};
use resonance_control::methods::song::TracksView;
use resonance_control::{MutationAck, Request, Response};

const TRACK: u64 = 1;

fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    // The history only records once the project has a path on disk.
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-edit-undo.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app
}

fn roundtrip(app: &mut Resonance, req: Request) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request: req,
        reply,
    })));
    rx.try_recv().expect("one reply per request")
}

fn call(app: &mut Resonance, method: &str, params: serde_json::Value) -> Response {
    roundtrip(app, Request::new(1, method, &params).expect("params serialize"))
}

fn status(app: &mut Resonance) -> EditStatus {
    roundtrip(app, Request::without_params(99, "edit.status"))
        .result()
        .expect("edit.status succeeds")
}

fn undo(app: &mut Resonance) -> UndoResult {
    roundtrip(app, Request::without_params(98, "edit.undo"))
        .result()
        .expect("edit.undo succeeds")
}

fn redo(app: &mut Resonance) -> RedoResult {
    roundtrip(app, Request::without_params(97, "edit.redo"))
        .result()
        .expect("edit.redo succeeds")
}

fn volume_db(app: &mut Resonance) -> f32 {
    let view: TracksView = roundtrip(app, Request::without_params(96, "song.tracks"))
        .result()
        .expect("song.tracks succeeds");
    view.tracks
        .iter()
        .find(|t| t.summary.id.0 == TRACK)
        .expect("track")
        .summary
        .volume_db
}

fn set_volume(app: &mut Resonance, db: f32) {
    let _: MutationAck = call(
        app,
        "mixer.set_volume_db",
        serde_json::json!({"track_id": TRACK, "volume_db": db}),
    )
    .result()
    .expect("mixer.set_volume_db succeeds");
}

#[test]
fn status_reports_an_empty_history_without_touching_it() {
    let mut app = app();
    let s = status(&mut app);
    assert!(!s.can_undo);
    assert!(!s.can_redo);
    assert_eq!(s.undo_label, None);
    assert_eq!(s.redo_label, None);
}

#[test]
fn a_control_edit_can_be_undone_and_redone() {
    let mut app = app();
    set_volume(&mut app, -6.0);
    assert!((volume_db(&mut app) - -6.0).abs() < 1e-4);

    // The edit is visible in the history BEFORE undoing it — an agent
    // must never have to undo blindly to find out what is there.
    let s = status(&mut app);
    assert!(s.can_undo, "the control edit is on the stack");
    assert_eq!(s.undo_label.as_deref(), Some("track volume"));
    assert!(!s.can_redo);

    let before = app.revision();
    let result = undo(&mut app);
    assert_eq!(result.undone.as_deref(), Some("track volume"));
    assert!(result.revision > before, "an undo is a committed change");
    assert!(
        volume_db(&mut app).abs() < 1e-4,
        "the prior state is restored"
    );
    // The result carries the history's new state, so no follow-up call
    // is needed to learn that a redo is now available.
    assert!(!result.status.can_undo);
    assert!(result.status.can_redo);
    assert_eq!(result.status.redo_label.as_deref(), Some("track volume"));

    let result = redo(&mut app);
    assert_eq!(result.redone.as_deref(), Some("track volume"));
    assert!((volume_db(&mut app) - -6.0).abs() < 1e-4, "the edit comes back");
    assert!(result.status.can_undo);
    assert!(!result.status.can_redo);
}

#[test]
fn labels_distinguish_different_kinds_of_edit() {
    let mut app = app();
    set_volume(&mut app, -3.0);
    assert_eq!(status(&mut app).undo_label.as_deref(), Some("track volume"));

    let _: MutationAck = call(
        &mut app,
        "track.rename",
        serde_json::json!({"track_id": TRACK, "name": "Bass"}),
    )
    .result()
    .expect("track.rename succeeds");
    assert_eq!(
        status(&mut app).undo_label.as_deref(),
        Some("rename track"),
        "the newest entry is what undo would back out"
    );

    // Undoing the rename exposes the volume edit underneath it.
    assert_eq!(undo(&mut app).undone.as_deref(), Some("rename track"));
    assert_eq!(status(&mut app).undo_label.as_deref(), Some("track volume"));
}

/// Undoing with an empty stack is an answer, not a failure.
#[test]
fn undo_and_redo_on_an_empty_history_are_clean_no_ops() {
    let mut app = app();
    let before = app.revision();

    let result = undo(&mut app);
    assert_eq!(result.undone, None);
    assert_eq!(result.revision, before, "nothing was committed");
    assert!(!result.status.can_undo);

    let result = redo(&mut app);
    assert_eq!(result.redone, None);
    assert_eq!(result.revision, before);
    assert!(!result.status.can_redo);
}

/// A new edit invalidates the redo stack, exactly as in the GUI.
#[test]
fn a_new_edit_clears_the_redo_stack() {
    let mut app = app();
    set_volume(&mut app, -6.0);
    assert!(undo(&mut app).status.can_redo);

    set_volume(&mut app, -2.0);
    let s = status(&mut app);
    assert!(!s.can_redo, "the new edit dropped the redo entry");
    assert_eq!(s.undo_label.as_deref(), Some("track volume"));
}

#[test]
fn edit_methods_need_an_open_project() {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(false);

    for method in ["edit.status", "edit.undo", "edit.redo"] {
        let error = roundtrip(&mut app, Request::without_params(1, method))
            .error
            .unwrap_or_else(|| panic!("{method} should be busy without a project"));
        assert_eq!(
            error.kind(),
            resonance_control::ErrorKind::Busy,
            "for {method}"
        );
    }
}

#[test]
fn every_edit_method_is_advertised_in_the_handshake() {
    let capabilities = resonance_control::methods::capabilities();
    for method in ["edit.undo", "edit.redo", "edit.status"] {
        assert!(capabilities.contains(&method), "{method} missing");
    }
}
