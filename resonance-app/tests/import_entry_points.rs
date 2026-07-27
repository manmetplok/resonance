//! MIDI Import — entry points (epic #28, todo #509).
//!
//! Two ways start an import: dragging a `.mid` onto the window (the
//! window file-drop subscription) and the chrome **Import…** affordance.
//! Both converge on `ImportMessage::Open` / `FileDropped`. The window
//! subscription itself can't be exercised headless, so these tests pin the
//! two halves it relies on: the `is_midi_path` filter that decides which
//! drops count, and the reducer behaviour for the messages it emits
//! (`HoverFile` / `HoverLeft` / `FileDropped`), driven through the real
//! `Resonance::update` so the startup gate is exercised too.

use std::path::Path;

use resonance_app::message::{ImportMessage, Message};
use resonance_app::state::ImportStage;
use resonance_app::update::import::is_midi_path;
use resonance_app::Resonance;

/// App with an active project so the startup-modal gate doesn't swallow
/// `Import` messages (parity with `tests/import_dialog.rs`).
fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app
}

fn send(app: &mut Resonance, m: ImportMessage) {
    let _ = app.update(Message::Import(m));
}

// -- is_midi_path: only .mid/.midi start an import -------------------------

#[test]
fn is_midi_path_accepts_mid_and_midi_any_case() {
    assert!(is_midi_path(Path::new("/tmp/song.mid")));
    assert!(is_midi_path(Path::new("/tmp/song.midi")));
    assert!(is_midi_path(Path::new("/tmp/SONG.MID")));
    assert!(is_midi_path(Path::new("/tmp/Song.MidI")));
}

#[test]
fn is_midi_path_rejects_other_files() {
    assert!(!is_midi_path(Path::new("/tmp/track.wav")));
    assert!(!is_midi_path(Path::new("/tmp/notes.txt")));
    assert!(!is_midi_path(Path::new("/tmp/midi")), "no extension");
    assert!(!is_midi_path(Path::new("/tmp/song.mid.bak")));
    assert!(!is_midi_path(Path::new("/tmp/.midi")), "dotfile, no stem");
}

// -- drop entry point -----------------------------------------------------

#[test]
fn dropping_a_file_opens_the_modal_when_closed() {
    let mut app = app_with_project();
    assert!(app.test_import_dialog().is_none(), "modal starts closed");

    send(&mut app, ImportMessage::FileDropped("/tmp/dropped.mid".into()));

    let d = app.test_import_dialog().expect("drop opens the modal");
    assert_eq!(d.stage, ImportStage::Parsing);
    assert_eq!(
        d.source_path.as_deref(),
        Some(Path::new("/tmp/dropped.mid"))
    );
    assert!(
        !d.opened_by_hover,
        "a real drop is a committed import, not a hover"
    );
}

// -- hover affordance -----------------------------------------------------

#[test]
fn hovering_a_midi_file_opens_the_drop_stage() {
    let mut app = app_with_project();
    send(&mut app, ImportMessage::HoverFile);

    let d = app.test_import_dialog().expect("hover surfaces the drop target");
    assert_eq!(d.stage, ImportStage::Drop);
    assert!(d.source_path.is_none());
    assert!(d.opened_by_hover);
}

#[test]
fn hover_left_closes_a_hover_opened_empty_dialog() {
    let mut app = app_with_project();
    send(&mut app, ImportMessage::HoverFile);
    assert!(app.test_import_dialog().is_some());

    send(&mut app, ImportMessage::HoverLeft);
    assert!(
        app.test_import_dialog().is_none(),
        "dragging away dismisses a dialog the hover itself opened"
    );
}

#[test]
fn hover_left_keeps_a_button_opened_dialog() {
    let mut app = app_with_project();
    // Opened deliberately via the chrome Import… affordance.
    send(&mut app, ImportMessage::Open);

    send(&mut app, ImportMessage::HoverLeft);
    let d = app
        .test_import_dialog()
        .expect("a deliberately-opened dialog survives a stray drag-out");
    assert_eq!(d.stage, ImportStage::Drop);
}

#[test]
fn hover_does_not_disturb_an_in_flight_dialog() {
    let mut app = app_with_project();
    send(&mut app, ImportMessage::Open);
    send(&mut app, ImportMessage::FileChosen("/tmp/song.mid".into()));
    assert_eq!(app.test_import_dialog().unwrap().stage, ImportStage::Parsing);

    // A file dragged over while a parse is already underway must not reset
    // the dialog back to the Drop stage.
    send(&mut app, ImportMessage::HoverFile);
    assert_eq!(app.test_import_dialog().unwrap().stage, ImportStage::Parsing);
}

#[test]
fn hover_left_keeps_a_dialog_that_already_took_a_drop() {
    let mut app = app_with_project();
    send(&mut app, ImportMessage::HoverFile);
    send(&mut app, ImportMessage::FileDropped("/tmp/song.mid".into()));
    assert_eq!(app.test_import_dialog().unwrap().stage, ImportStage::Parsing);

    // Some platforms fire FilesHoveredLeft after a successful drop; that
    // must not tear down the parsing dialog.
    send(&mut app, ImportMessage::HoverLeft);
    assert_eq!(app.test_import_dialog().unwrap().stage, ImportStage::Parsing);
}

// -- gating ---------------------------------------------------------------

#[test]
fn entry_points_are_gated_until_a_project_is_open() {
    // No active project → the startup modal owns the screen; neither entry
    // point may open the import overlay.
    let (mut app, _task) = Resonance::new();
    send(&mut app, ImportMessage::HoverFile);
    assert!(app.test_import_dialog().is_none(), "hover gated");

    send(&mut app, ImportMessage::FileDropped("/tmp/song.mid".into()));
    assert!(app.test_import_dialog().is_none(), "drop gated");
}
