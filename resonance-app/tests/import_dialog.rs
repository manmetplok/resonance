//! MIDI Import modal — shell + state plumbing (epic #28, todo #505).
//!
//! These drive the real reducer (`update::import`) through
//! `Resonance::update` to pin the open/close lifecycle and the
//! review-stage field setters. The per-stage view bodies and the actual
//! parse/import land in follow-up todos (doc #158); here we only assert
//! that the dialog state transitions the way the message contract says.

use resonance_app::message::{ImportMessage, Message};
use resonance_app::state::{
    ImportStage, ImportSummary, ParsedImport, PlacementMode, PlacementStart, TempoAlignment,
    TempoChoice, TrackImportRow,
};
use resonance_app::Resonance;

/// Build an app with an active project so the startup-modal gate doesn't
/// swallow `Import` messages (the import modal is gated like other
/// auxiliary overlays until a project is open).
fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app
}

fn send(app: &mut Resonance, m: ImportMessage) {
    let _ = app.update(Message::Import(m));
}

fn row(name: &str, channel: u8, note_count: usize) -> TrackImportRow {
    TrackImportRow {
        selected: true,
        name: name.to_string(),
        channel,
        note_count,
        pitch_min: Some(48),
        pitch_max: Some(72),
        is_conductor: false,
        preview: Vec::new(),
    }
}

fn parsed(rows: Vec<TrackImportRow>, tempo_conflict: bool) -> ParsedImport {
    let total_notes = rows.iter().map(|r| r.note_count).sum();
    ParsedImport {
        summary: ImportSummary {
            file_name: "song.mid".to_string(),
            track_count: rows.len(),
            total_notes,
            file_tempo_bpm: Some(140.0),
            tempo_conflict,
        },
        rows,
    }
}

#[test]
fn open_shows_modal_at_drop_stage() {
    let mut app = app_with_project();
    assert!(app.test_import_dialog().is_none(), "modal starts closed");

    send(&mut app, ImportMessage::Open);
    let d = app.test_import_dialog().expect("modal open after Open");
    assert_eq!(d.stage, ImportStage::Drop);
    assert!(d.source_path.is_none());
    assert!(d.rows.is_empty());
    // Sensible defaults.
    assert_eq!(d.tempo_choice, TempoChoice::KeepProject);
    assert_eq!(d.placement.start, PlacementStart::Bar1);
    assert_eq!(d.placement.mode, PlacementMode::NewTracks);
}

#[test]
fn cancel_closes_modal() {
    let mut app = app_with_project();
    send(&mut app, ImportMessage::Open);
    assert!(app.test_import_dialog().is_some());

    send(&mut app, ImportMessage::Cancel);
    assert!(app.test_import_dialog().is_none(), "Cancel closes the modal");
}

#[test]
fn choosing_a_file_moves_to_parsing() {
    let mut app = app_with_project();
    send(&mut app, ImportMessage::Open);

    send(
        &mut app,
        ImportMessage::FileChosen("/tmp/song.mid".into()),
    );
    let d = app.test_import_dialog().unwrap();
    assert_eq!(d.stage, ImportStage::Parsing);
    assert_eq!(d.source_path.as_deref(), Some(std::path::Path::new("/tmp/song.mid")));
}

#[test]
fn dropping_a_file_moves_to_parsing() {
    let mut app = app_with_project();
    send(&mut app, ImportMessage::Open);

    send(
        &mut app,
        ImportMessage::FileDropped("/tmp/dropped.mid".into()),
    );
    let d = app.test_import_dialog().unwrap();
    assert_eq!(d.stage, ImportStage::Parsing);
    assert_eq!(
        d.source_path.as_deref(),
        Some(std::path::Path::new("/tmp/dropped.mid"))
    );
}

#[test]
fn parse_ok_without_conflict_goes_to_review() {
    let mut app = app_with_project();
    send(&mut app, ImportMessage::Open);
    send(&mut app, ImportMessage::FileChosen("/tmp/song.mid".into()));

    let rows = vec![row("Lead", 0, 12), row("Bass", 1, 8)];
    send(
        &mut app,
        ImportMessage::ParseCompleted(Ok(parsed(rows, false))),
    );

    let d = app.test_import_dialog().unwrap();
    assert_eq!(d.stage, ImportStage::Review);
    assert_eq!(d.rows.len(), 2);
    assert_eq!(d.summary.as_ref().unwrap().total_notes, 20);
    assert!(d.error.is_none());
}

#[test]
fn parse_ok_with_conflict_goes_to_tempo_conflict() {
    let mut app = app_with_project();
    send(&mut app, ImportMessage::Open);
    send(&mut app, ImportMessage::FileChosen("/tmp/song.mid".into()));

    send(
        &mut app,
        ImportMessage::ParseCompleted(Ok(parsed(vec![row("Lead", 0, 4)], true))),
    );

    let d = app.test_import_dialog().unwrap();
    assert_eq!(d.stage, ImportStage::TempoConflict);
}

#[test]
fn parse_err_goes_to_error_stage_with_reason() {
    let mut app = app_with_project();
    send(&mut app, ImportMessage::Open);
    send(&mut app, ImportMessage::FileChosen("/tmp/bad.mid".into()));

    send(
        &mut app,
        ImportMessage::ParseCompleted(Err("not a MIDI file".to_string())),
    );

    let d = app.test_import_dialog().unwrap();
    assert_eq!(d.stage, ImportStage::Error);
    assert_eq!(d.error.as_deref(), Some("not a MIDI file"));
}

#[test]
fn toggle_track_flips_only_that_row() {
    let mut app = app_with_project();
    send(&mut app, ImportMessage::Open);
    send(
        &mut app,
        ImportMessage::ParseCompleted(Ok(parsed(vec![row("A", 0, 1), row("B", 1, 1)], false))),
    );

    send(&mut app, ImportMessage::ToggleTrack(0));
    let d = app.test_import_dialog().unwrap();
    assert!(!d.rows[0].selected, "row 0 toggled off");
    assert!(d.rows[1].selected, "row 1 untouched");

    // Out-of-range index is a no-op, not a panic.
    send(&mut app, ImportMessage::ToggleTrack(99));
    assert!(app.test_import_dialog().is_some());
}

#[test]
fn set_all_tracks_applies_to_every_row() {
    let mut app = app_with_project();
    send(&mut app, ImportMessage::Open);
    send(
        &mut app,
        ImportMessage::ParseCompleted(Ok(parsed(vec![row("A", 0, 1), row("B", 1, 1)], false))),
    );

    send(&mut app, ImportMessage::SetAllTracks(false));
    let d = app.test_import_dialog().unwrap();
    assert!(d.rows.iter().all(|r| !r.selected));

    send(&mut app, ImportMessage::SetAllTracks(true));
    let d = app.test_import_dialog().unwrap();
    assert!(d.rows.iter().all(|r| r.selected));
}

#[test]
fn rename_track_updates_the_destination_name() {
    let mut app = app_with_project();
    send(&mut app, ImportMessage::Open);
    send(
        &mut app,
        ImportMessage::ParseCompleted(Ok(parsed(vec![row("Track 1", 0, 1)], false))),
    );

    send(
        &mut app,
        ImportMessage::RenameTrack(0, "Strings".to_string()),
    );
    assert_eq!(app.test_import_dialog().unwrap().rows[0].name, "Strings");
}

#[test]
fn tempo_placement_and_alignment_setters_stick() {
    let mut app = app_with_project();
    send(&mut app, ImportMessage::Open);

    send(&mut app, ImportMessage::SetTempoChoice(TempoChoice::AdoptFile));
    send(
        &mut app,
        ImportMessage::SetPlacementStart(PlacementStart::Playhead),
    );
    send(
        &mut app,
        ImportMessage::SetPlacementMode(PlacementMode::MergeIntoSelected),
    );
    send(
        &mut app,
        ImportMessage::SetConflictAlignment(TempoAlignment::MatchTime),
    );

    let d = app.test_import_dialog().unwrap();
    assert_eq!(d.tempo_choice, TempoChoice::AdoptFile);
    assert_eq!(d.placement.start, PlacementStart::Playhead);
    assert_eq!(d.placement.mode, PlacementMode::MergeIntoSelected);
    assert_eq!(d.tempo_alignment, TempoAlignment::MatchTime);
}

#[test]
fn import_messages_are_gated_until_a_project_is_open() {
    // No active project → the startup modal owns the screen and the
    // import overlay must not open (parity with Open Settings / Add Track).
    let (mut app, _task) = Resonance::new();
    // A fresh `Resonance` has no active project (the startup modal is up).
    send(&mut app, ImportMessage::Open);
    assert!(
        app.test_import_dialog().is_none(),
        "Open must be gated while no project is active"
    );
}
