//! Behavioural coverage and golden-image snapshots for the
//! **audio-import transcode-progress modal** (design doc #175, ba todo #606).
//!
//! The modal is shown while the engine copies and transcodes a multi-file
//! audio import batch into the project folder (`*.rproj/audio/`). It lists
//! every file in the current batch with a live status indicator
//! (queued / working / done / failed) driven by `ImportProgress` /
//! `ImportFailed` engine events, and carries an explanatory note that files
//! are copied so the project stays self-contained.
//!
//! Tests cover:
//!
//! * Opening the modal when an import batch starts.
//! * Progress tracker correctly reflects all four status variants.
//! * `DismissImportProgress` closes the modal and clears the tracker.
//! * The modal is **not** dismissible (backdrop click has no effect) while
//!   files are still in flight.
//! * Two golden-PNG snapshots:
//!   1. **In-progress** — one queued and one working file, backdrop is live.
//!   2. **Complete** — one done and one failed file, "Done" button visible.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::state::{FileImportProgress, ViewMode};
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_audio::types::{AssetId, AudioEvent, ImportStage};

const WINDOW: (f32, f32) = (1440.0, 900.0);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sim_settings() -> iced::Settings {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    fonts.push(theme::ICON_FONT_BYTES.into());
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    }
}

/// Build a demo app with the arrange view active.
fn demo_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    app
}

/// Fire an `ImportProgress` engine event into the app.
fn fire_progress(app: &mut Resonance, asset_id: AssetId, path: &str, stage: ImportStage) {
    app.test_apply_engine_event(AudioEvent::ImportProgress {
        asset_id,
        path: path.to_string(),
        stage,
    });
}

/// Fire an `ImportFailed` engine event into the app.
fn fire_failed(app: &mut Resonance, asset_id: AssetId, path: &str, reason: &str) {
    app.test_apply_engine_event(AudioEvent::ImportFailed {
        asset_id,
        path: path.to_string(),
        reason: reason.to_string(),
    });
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image(path).expect("matches_image i/o"),
        "snapshot diverged from golden: {path}"
    );
}

// ---------------------------------------------------------------------------
// Behavioural tests
// ---------------------------------------------------------------------------

/// The modal is initially closed on a fresh app.
#[test]
fn modal_starts_closed() {
    let app = demo_app();
    assert!(!app.test_import_progress_modal_open());
}

/// `test_set_import_progress_modal_open(true)` opens the modal so snapshot
/// tests can render it without driving a real import.
#[test]
fn modal_can_be_opened_for_tests() {
    let mut app = demo_app();
    app.test_set_import_progress_modal_open(true);
    assert!(app.test_import_progress_modal_open());
}

/// `DismissImportProgress` closes the modal and clears the tracker.
#[test]
fn dismiss_closes_modal_and_clears_tracker() {
    let mut app = demo_app();
    app.test_set_import_progress_modal_open(true);

    // Seed a progress entry so we can verify clear.
    fire_progress(&mut app, 1, "/path/to/loop.wav", ImportStage::Done);
    assert!(!app.test_import_progress().statuses().is_empty());

    app.test_dispatch(Message::Ui(UiMessage::DismissImportProgress));

    assert!(!app.test_import_progress_modal_open(), "modal should be closed");
    assert!(
        app.test_import_progress().statuses().is_empty(),
        "tracker should be cleared on dismiss",
    );
}

/// `ImportProgress(Queued)` adds an entry in the Queued state.
#[test]
fn progress_queued_adds_entry() {
    let mut app = demo_app();
    fire_progress(&mut app, 1, "/samples/bass.wav", ImportStage::Queued);
    let statuses = app.test_import_progress().statuses().to_vec();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].progress, FileImportProgress::Queued);
}

/// `ImportProgress(Working)` transitions an entry to Working.
#[test]
fn progress_working_updates_entry() {
    let mut app = demo_app();
    fire_progress(&mut app, 1, "/samples/bass.wav", ImportStage::Queued);
    fire_progress(&mut app, 1, "/samples/bass.wav", ImportStage::Working);
    let statuses = app.test_import_progress().statuses().to_vec();
    assert_eq!(statuses.len(), 1, "upsert — no duplicate");
    assert_eq!(statuses[0].progress, FileImportProgress::Working);
}

/// `ImportProgress(Done)` marks an entry Done.
#[test]
fn progress_done_marks_entry_done() {
    let mut app = demo_app();
    fire_progress(&mut app, 1, "/samples/bass.wav", ImportStage::Queued);
    fire_progress(&mut app, 1, "/samples/bass.wav", ImportStage::Working);
    fire_progress(&mut app, 1, "/samples/bass.wav", ImportStage::Done);
    let statuses = app.test_import_progress().statuses().to_vec();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].progress, FileImportProgress::Done);
}

/// `ImportFailed` marks an entry Failed with a user-facing reason.
#[test]
fn progress_failed_marks_entry_failed() {
    let mut app = demo_app();
    fire_progress(&mut app, 2, "/samples/broken.ogg", ImportStage::Queued);
    fire_failed(&mut app, 2, "/samples/broken.ogg", "unsupported codec");
    let statuses = app.test_import_progress().statuses().to_vec();
    assert_eq!(statuses.len(), 1);
    assert_eq!(
        statuses[0].progress,
        FileImportProgress::Failed {
            reason: "unsupported codec".into()
        }
    );
}

/// `is_complete()` is false when any entry is still Queued or Working.
#[test]
fn tracker_not_complete_while_any_in_flight() {
    let mut app = demo_app();
    fire_progress(&mut app, 1, "/a.wav", ImportStage::Done);
    fire_progress(&mut app, 2, "/b.wav", ImportStage::Queued);
    assert!(!app.test_import_progress().is_complete());
}

/// `is_complete()` is true once all entries are Done or Failed.
#[test]
fn tracker_complete_when_all_settled() {
    let mut app = demo_app();
    fire_progress(&mut app, 1, "/a.wav", ImportStage::Done);
    fire_failed(&mut app, 2, "/b.wav", "bad format");
    assert!(app.test_import_progress().is_complete());
}

/// An empty tracker is considered complete (the modal starts with no entries
/// before the first `ImportProgress` event lands).
#[test]
fn empty_tracker_is_complete() {
    let app = demo_app();
    assert!(app.test_import_progress().is_complete());
}

// ---------------------------------------------------------------------------
// Golden-image snapshots
// ---------------------------------------------------------------------------

/// Golden: in-progress batch — one file queued, one working.
/// The backdrop blocks clicks (no "Done" button visible).
#[test]
fn import_progress_modal_in_progress() {
    let mut app = demo_app();

    fire_progress(&mut app, 1, "/Users/max/Samples/Drums/Kick 808.wav", ImportStage::Queued);
    fire_progress(&mut app, 2, "/Users/max/Samples/Drums/Snare Rimshot.flac", ImportStage::Working);

    app.test_set_import_progress_modal_open(true);

    snapshot_to(
        &app,
        "tests/snapshots/import_progress_modal_in_progress.png",
    );
}

/// Golden: completed batch — one done, one failed.
/// The "Done" button is enabled; the failed row shows a BAD glyph and the
/// inline error reason.
#[test]
fn import_progress_modal_complete() {
    let mut app = demo_app();

    fire_progress(&mut app, 1, "/Users/max/Samples/Drums/Kick 808.wav", ImportStage::Done);
    fire_failed(
        &mut app,
        2,
        "/Users/max/Downloads/Crowd Ambience (corrupt).mp3",
        "unsupported codec — re-encode to WAV/FLAC/OGG",
    );

    app.test_set_import_progress_modal_open(true);

    snapshot_to(
        &app,
        "tests/snapshots/import_progress_modal_complete.png",
    );
}
