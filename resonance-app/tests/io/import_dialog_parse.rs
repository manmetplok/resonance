//! A dropped or chosen MIDI file is actually parsed, and the Drop stage
//! offers a file chooser (code review VIEW-25).
//!
//! `FileDropped` used to set the stage to `Parsing` and return
//! `Task::none()`: nothing ever produced `ParseCompleted`, so the modal sat
//! on "Parsing…" forever, and its "or choose one" prompt had no chooser.

use std::path::PathBuf;

use resonance_app::message::{ImportMessage, Message};
use resonance_app::state::ImportStage;
use resonance_app::update::import::parse_import_file;
use resonance_app::Resonance;
use resonance_audio::midi_io::write_midi_file;
use resonance_audio::types::MidiNote;

fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app
}

/// Write a small one-track SMF to a per-test temp path.
fn midi_file(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "resonance-import-parse-{tag}-{}.mid",
        std::process::id()
    ));
    let notes: Vec<MidiNote> = (0..4)
        .map(|i| MidiNote {
            note: 60 + i as u8,
            velocity: 0.8,
            start_tick: i * 480,
            duration_ticks: 480,
        })
        .collect();
    write_midi_file(&path, &notes).expect("write test MIDI file");
    path
}

#[test]
fn a_drop_spawns_the_parse() {
    let mut app = app_with_project();
    let path = midi_file("spawn");
    let task = app.update(Message::Import(ImportMessage::FileDropped(path)));
    assert_eq!(
        app.test_import_dialog().map(|d| d.stage),
        Some(ImportStage::Parsing)
    );
    assert!(task.units() > 0, "a drop must start a parse task, not Task::none()");
}

#[test]
fn the_parse_reports_the_file() {
    let path = midi_file("summary");
    let parsed = parse_import_file(&path, 120.0).expect("the file parses");
    assert_eq!(parsed.summary.total_notes, 4);
    assert!(parsed.summary.file_name.ends_with(".mid"));
    let importable: Vec<_> = parsed.rows.iter().filter(|r| !r.is_conductor).collect();
    assert_eq!(importable.len(), 1);
    assert_eq!(importable[0].note_count, 4);
    assert!(importable[0].selected, "a track with notes starts selected");
}

#[test]
fn a_parse_for_a_superseded_file_is_ignored() {
    let mut app = app_with_project();
    let first = midi_file("first");
    let second = midi_file("second");
    let _ = app.update(Message::Import(ImportMessage::FileDropped(first.clone())));
    let _ = app.update(Message::Import(ImportMessage::FileDropped(second.clone())));

    // The first file's parse lands late: the dialog is on the second file.
    let _ = app.update(Message::Import(ImportMessage::Parsed {
        path: first.clone(),
        result: parse_import_file(&first, 120.0),
    }));
    assert_eq!(
        app.test_import_dialog().map(|d| d.stage),
        Some(ImportStage::Parsing),
        "a stale parse must not advance the dialog"
    );

    let _ = app.update(Message::Import(ImportMessage::Parsed {
        path: second.clone(),
        result: parse_import_file(&second, 120.0),
    }));
    let dialog = app.test_import_dialog().expect("dialog open");
    assert_eq!(dialog.stage, ImportStage::Review);
    assert_eq!(dialog.rows.iter().filter(|r| !r.is_conductor).count(), 1);
}

#[test]
fn a_broken_file_lands_on_the_error_stage() {
    let mut app = app_with_project();
    let path = std::env::temp_dir().join(format!(
        "resonance-import-parse-broken-{}.mid",
        std::process::id()
    ));
    std::fs::write(&path, b"not a midi file").expect("write junk");
    let _ = app.update(Message::Import(ImportMessage::FileDropped(path.clone())));
    let _ = app.update(Message::Import(ImportMessage::Parsed {
        path: path.clone(),
        result: parse_import_file(&path, 120.0),
    }));
    let dialog = app.test_import_dialog().expect("dialog open");
    assert_eq!(dialog.stage, ImportStage::Error);
    assert!(dialog.error.is_some());
}

#[test]
fn the_drop_stage_offers_a_chooser() {
    let mut app = app_with_project();
    let _ = app.update(Message::Import(ImportMessage::Open));
    let task = app.update(Message::Import(ImportMessage::Choose));
    assert!(task.units() > 0, "Choose must open the file dialog");
}

/// Golden: the Drop stage with its chooser button.
#[test]
fn import_dialog_drop_stage_golden() {
    use iced::Size;
    use iced_test::simulator::Simulator;
    use resonance_app::state::ViewMode;
    use resonance_app::{demo, theme};

    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Import(ImportMessage::Open));

    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = vec![theme::ICON_FONT_BYTES.into()];
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    let settings = iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    };
    let mut ui = Simulator::with_size(settings, Size::new(1440.0, 900.0), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    crate::common::assert_golden(&snap, "tests/snapshots/import_dialog_drop.png");
}
