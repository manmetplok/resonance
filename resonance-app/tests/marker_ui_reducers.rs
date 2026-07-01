//! Reducer coverage for the transient marker-interaction messages
//! (`MarkerUiMessage`, todo #369): selection, the right-click context menu,
//! and the inline rename. These drive `update/marker_ui.rs` through the
//! public `Resonance::update` entry point. The rename commit edits the
//! persisted marker name and is classified `Record`, so it also asserts an
//! undo entry results; every other variant is pure view state (`Skip`).

use resonance_app::message::{MarkerMessage, MarkerUiMessage, Message};
use resonance_app::state::ArrangementMarker;
use resonance_app::undo::{classify, UndoAction};
use resonance_app::Resonance;

const SAMPLE_RATE: u32 = 48_000;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    // A saved path is required before the undo history records anything
    // (see `can_record_undo`), so the rename-undo round trip can restore.
    app.test_set_project_path(std::path::PathBuf::from("/tmp/markers-test.rsz"));
    app.test_set_sample_rate(SAMPLE_RATE);
    app
}

fn seed(app: &mut Resonance, name: &str, start: u64) -> u64 {
    app.test_add_marker(ArrangementMarker::new_point(
        start.max(1),
        name.to_string(),
        [10, 20, 30],
        start,
    ))
}

// ---------------- Selection ----------------

#[test]
fn select_sets_and_clears_selection() {
    let mut app = app();
    let id = seed(&mut app, "Intro", 48_000);

    let _ = app.update(Message::MarkerUi(MarkerUiMessage::Select(Some(id))));
    assert_eq!(app.test_selected_marker_id(), Some(id));

    let _ = app.update(Message::MarkerUi(MarkerUiMessage::Select(None)));
    assert_eq!(app.test_selected_marker_id(), None);
}

#[test]
fn select_dismisses_an_open_menu() {
    let mut app = app();
    let id = seed(&mut app, "Intro", 48_000);

    let _ = app.update(Message::MarkerUi(MarkerUiMessage::OpenMenu {
        id,
        x: 12.0,
        y: 20.0,
    }));
    assert!(app.test_marker_menu().is_some());

    let _ = app.update(Message::MarkerUi(MarkerUiMessage::Select(Some(id))));
    assert!(app.test_marker_menu().is_none());
}

// ---------------- Context menu ----------------

#[test]
fn open_menu_records_anchor_and_selects() {
    let mut app = app();
    let id = seed(&mut app, "Verse", 96_000);

    let _ = app.update(Message::MarkerUi(MarkerUiMessage::OpenMenu {
        id,
        x: 40.0,
        y: 18.0,
    }));

    let menu = app.test_marker_menu().expect("menu open");
    assert_eq!(menu.marker_id, id);
    assert_eq!(menu.x, 40.0);
    assert_eq!(menu.y, 18.0);
    assert_eq!(app.test_selected_marker_id(), Some(id));
}

#[test]
fn open_menu_ignores_missing_marker() {
    let mut app = app();
    let _ = app.update(Message::MarkerUi(MarkerUiMessage::OpenMenu {
        id: 999,
        x: 1.0,
        y: 2.0,
    }));
    assert!(app.test_marker_menu().is_none());
}

#[test]
fn close_menu_clears_it() {
    let mut app = app();
    let id = seed(&mut app, "Verse", 96_000);
    let _ = app.update(Message::MarkerUi(MarkerUiMessage::OpenMenu {
        id,
        x: 0.0,
        y: 0.0,
    }));
    let _ = app.update(Message::MarkerUi(MarkerUiMessage::CloseMenu));
    assert!(app.test_marker_menu().is_none());
}

// ---------------- Inline rename ----------------

#[test]
fn begin_rename_seeds_buffer_from_name_and_closes_menu() {
    let mut app = app();
    let id = seed(&mut app, "Chorus", 144_000);
    let _ = app.update(Message::MarkerUi(MarkerUiMessage::OpenMenu {
        id,
        x: 5.0,
        y: 6.0,
    }));

    let _ = app.update(Message::MarkerUi(MarkerUiMessage::BeginRename {
        id,
        x: 5.0,
        y: 6.0,
    }));

    let rename = app.test_marker_rename().expect("rename active");
    assert_eq!(rename.marker_id, id);
    assert_eq!(rename.text, "Chorus");
    // Opening the rename dismisses the menu.
    assert!(app.test_marker_menu().is_none());
}

#[test]
fn rename_changed_updates_buffer() {
    let mut app = app();
    let id = seed(&mut app, "Chorus", 144_000);
    let _ = app.update(Message::MarkerUi(MarkerUiMessage::BeginRename {
        id,
        x: 0.0,
        y: 0.0,
    }));

    let _ = app.update(Message::MarkerUi(MarkerUiMessage::RenameChanged(
        "Bridge".into(),
    )));
    assert_eq!(app.test_marker_rename().unwrap().text, "Bridge");
}

#[test]
fn commit_rename_applies_edit_and_is_undoable() {
    let mut app = app();
    let id = seed(&mut app, "Chorus", 144_000);
    let _ = app.update(Message::MarkerUi(MarkerUiMessage::BeginRename {
        id,
        x: 0.0,
        y: 0.0,
    }));
    let _ = app.update(Message::MarkerUi(MarkerUiMessage::RenameChanged(
        "  Bridge  ".into(),
    )));
    let _ = app.update(Message::MarkerUi(MarkerUiMessage::CommitRename));

    // Buffer cleared, name committed (trimmed).
    assert!(app.test_marker_rename().is_none());
    assert_eq!(app.test_markers().get(id).unwrap().name, "Bridge");

    // Undoable: walking back restores the original name.
    let _ = app.update(Message::Undo);
    assert_eq!(app.test_markers().get(id).unwrap().name, "Chorus");
}

#[test]
fn commit_rename_drops_empty_edit() {
    let mut app = app();
    let id = seed(&mut app, "Chorus", 144_000);
    let _ = app.update(Message::MarkerUi(MarkerUiMessage::BeginRename {
        id,
        x: 0.0,
        y: 0.0,
    }));
    let _ = app.update(Message::MarkerUi(MarkerUiMessage::RenameChanged("   ".into())));
    let _ = app.update(Message::MarkerUi(MarkerUiMessage::CommitRename));

    assert!(app.test_marker_rename().is_none());
    // Name unchanged — a blank edit is discarded.
    assert_eq!(app.test_markers().get(id).unwrap().name, "Chorus");
}

#[test]
fn cancel_rename_discards_edit() {
    let mut app = app();
    let id = seed(&mut app, "Chorus", 144_000);
    let _ = app.update(Message::MarkerUi(MarkerUiMessage::BeginRename {
        id,
        x: 0.0,
        y: 0.0,
    }));
    let _ = app.update(Message::MarkerUi(MarkerUiMessage::RenameChanged("X".into())));
    let _ = app.update(Message::MarkerUi(MarkerUiMessage::CancelRename));

    assert!(app.test_marker_rename().is_none());
    assert_eq!(app.test_markers().get(id).unwrap().name, "Chorus");
}

// ---------------- Undo classification ----------------

#[test]
fn transient_ui_messages_skip_undo() {
    for m in [
        MarkerUiMessage::Select(Some(1)),
        MarkerUiMessage::OpenMenu {
            id: 1,
            x: 0.0,
            y: 0.0,
        },
        MarkerUiMessage::CloseMenu,
        MarkerUiMessage::BeginRename {
            id: 1,
            x: 0.0,
            y: 0.0,
        },
        MarkerUiMessage::RenameChanged("a".into()),
        MarkerUiMessage::CancelRename,
    ] {
        assert!(matches!(
            classify(&Message::MarkerUi(m)),
            UndoAction::Skip
        ));
    }
}

#[test]
fn commit_rename_records_undo() {
    assert!(matches!(
        classify(&Message::MarkerUi(MarkerUiMessage::CommitRename)),
        UndoAction::Record
    ));
}

#[test]
fn drag_burst_collapses_to_one_undo_entry() {
    // A drag dispatches many `MoveStart` messages; because they coalesce
    // (todo #369), a single undo returns to the pre-drag position rather
    // than merely stepping back one pointer sample.
    let mut app = app();
    let id = seed(&mut app, "Intro", 48_000);

    for target in [96_000u64, 120_000, 144_000] {
        let _ = app.update(Message::Marker(MarkerMessage::MoveStart(id, target)));
    }
    assert_ne!(app.test_markers().get(id).unwrap().start_sample, 48_000);

    let _ = app.update(Message::Undo);
    assert_eq!(app.test_markers().get(id).unwrap().start_sample, 48_000);
}
