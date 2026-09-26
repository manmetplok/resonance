//! View state and async completions are no edits (code review VIEW-18).
//!
//! Selecting a drum-ribbon span, switching the Expression dock's curve,
//! pen or snap, and a vocal render finishing seconds after its edit all
//! fell through `classify` to `Record`: each one pushed an entry, wiped
//! the redo stack and (for the view state) marked the project dirty.

use resonance_app::compose::messages::{ExpressionMessage, VocalAudioReadyData};
use resonance_app::compose::vocal_svs::CurveKind;
use resonance_app::compose::{ComposeMessage, PenMode};
use resonance_app::message::{Message, TrackMessage};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::TrackType;

const TRACK: u64 = 1;
const DEF: u64 = 1;

/// An app with one edit undone, so the redo stack holds an entry, and a
/// clean dirty flag.
fn app_with_redo() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/view18-undo.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -6.0)));
    let _ = app.update(Message::Undo);
    assert!(app.test_undo_history().can_redo());
    app.test_set_dirty(false);
    app
}

fn assert_no_edit(msg: Message) {
    let mut app = app_with_redo();
    let revision = app.revision();
    let label = format!("{msg:?}");
    let _ = app.update(msg);
    assert!(app.test_undo_history().can_redo(), "{label} wiped redo");
    assert!(!app.is_dirty(), "{label} marked the project dirty");
    assert_eq!(app.revision(), revision, "{label} bumped the revision");
}

fn expression(msg: ExpressionMessage) -> Message {
    Message::Compose(ComposeMessage::Expression {
        definition_id: DEF,
        track_id: TRACK,
        msg,
    })
}

#[test]
fn ribbon_span_selection_is_no_edit() {
    assert_no_edit(Message::Compose(ComposeMessage::SelectArrangementEntry(Some(0))));
    assert_no_edit(Message::Compose(ComposeMessage::SelectArrangementEntry(None)));
}

#[test]
fn expression_tool_state_is_no_edit() {
    assert_no_edit(expression(ExpressionMessage::SelectCurve(CurveKind::Tension)));
    assert_no_edit(expression(ExpressionMessage::SetPenMode(PenMode::Line)));
    assert_no_edit(expression(ExpressionMessage::SetSnap(true)));
}

fn ready(epoch: u64) -> Message {
    Message::Compose(ComposeMessage::VocalAudioReady(Box::new(VocalAudioReadyData {
        definition_id: DEF,
        track_id: TRACK,
        wav_path: std::env::temp_dir().join("resonance-view18-missing.wav"),
        placements: Vec::new(),
        clip_name: "vocal".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        lead_ticks: 0,
        render_epoch: epoch,
        bpm: 120.0,
    })))
}

#[test]
fn superseded_vocal_completions_are_no_edit() {
    assert_no_edit(ready(99));
    assert_no_edit(Message::Compose(ComposeMessage::VocalAudioFailed {
        definition_id: DEF,
        track_id: TRACK,
        render_epoch: 99,
        error: "boom".into(),
    }));
}

/// An accepted render changes the project's clips, so it still marks the
/// project dirty and bumps the revision — but it is the tail of the edit
/// that queued it, not a new one, so the redo stack survives.
#[test]
fn accepted_vocal_render_keeps_redo_but_marks_dirty() {
    let mut app = app_with_redo();
    app.test_set_vocal_render_epoch(DEF, TRACK, 5);
    let revision = app.revision();
    let entries = app.test_undo_history().test_undo_entries().len();

    let _ = app.update(ready(5));

    assert!(app.test_undo_history().can_redo(), "a render completion wiped redo");
    assert_eq!(app.test_undo_history().test_undo_entries().len(), entries);
    assert!(app.is_dirty(), "installed vocal audio must mark the project dirty");
    assert!(app.revision() > revision, "installed vocal audio must bump the revision");
}
