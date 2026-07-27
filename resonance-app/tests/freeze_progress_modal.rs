//! Freeze progress modal (design doc #181, ba todo #582).
//!
//! While a freeze render is in flight the app shows the bounce-in-place
//! blocking overlay relabelled for freeze: snowflake + serif-italic
//! "Freezing "name"" title, the 14 px progress bar fed from the engine's
//! `FreezeProgress` fractions (mirrored by ba todo #575), a mono percent
//! caption — extended with a "track N / M" counter during a freeze-all /
//! freeze-selected batch — and a ghost Cancel that dispatches
//! `FreezeMessage::CancelFreeze` (the one message the freeze gate lets
//! through).
//!
//! Coverage:
//! - **Presence** (widget tree): the modal renders title, counter, and
//!   Cancel while a track is `Freezing`; absent when idle.
//! - **Gating** (update-driven): transport input is dropped mid-render;
//!   Cancel flows through and clears the render + batch.
//! - **Golden** (`tests/snapshots`): a single-track freeze at 42% and a
//!   freeze-all batch run showing the "track N / M" counter.

mod common;

use iced::Size;
use resonance_app::message::{FreezeMessage, Message, TransportMessage};
use resonance_app::state::{FreezeStatus, ViewMode};
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};

use iced_test::simulator::Simulator;

const WINDOW: (f32, f32) = (1440.0, 900.0);

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

/// Demo app pinned to the Arrange tab.
fn build_arrange_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    app
}

/// Mark track 1 ("Drums") as mid-render at the given fraction, as the
/// engine freeze-event mirror (ba todo #575) would.
fn set_freezing(app: &mut Resonance, fraction: f32) {
    app.test_set_freeze_status(1, FreezeStatus::Freezing { fraction });
}

/// Kick off a real freeze-all batch through the update path. Points the
/// project at a unique temp path first so `start_freeze` can derive the
/// sibling `<project>.freeze/` cache directory.
fn start_freeze_all_batch(app: &mut Resonance) {
    let dir = std::env::temp_dir().join(format!(
        "resonance_freeze_modal_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp project dir");
    app.test_set_project_path(dir.join("project.rproj"));
    app.test_dispatch(Message::Freeze(FreezeMessage::FreezeAllTracks));
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

// ---------------------------------------------------------------------
// Presence — the modal appears exactly while a render is in flight.
// ---------------------------------------------------------------------

#[test]
fn modal_renders_while_freezing() {
    let mut app = build_arrange_app();
    set_freezing(&mut app, 0.42);
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    ui.find("Freezing \"Drums\"")
        .expect("freeze modal must render the snowflake title with the track name");
    ui.find("Cancel")
        .expect("freeze modal must render the ghost Cancel button");
    ui.find("42%")
        .expect("freeze modal must render the mono percent caption");
}

#[test]
fn modal_absent_when_idle() {
    let app = build_arrange_app();
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    assert!(
        ui.find("Freezing \"Drums\"").is_err(),
        "no freeze modal may render while nothing is freezing"
    );
}

#[test]
fn batch_run_shows_track_counter() {
    let mut app = build_arrange_app();
    start_freeze_all_batch(&mut app);
    let queue = app
        .test_freeze_queue()
        .expect("FreezeAllTracks must queue a batch");
    let total = queue.total;
    assert!(
        total > 1,
        "demo content must yield a multi-track batch, got {total}"
    );
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let counter = format!("0% \u{00b7} track 1 / {total}");
    ui.find(counter.as_str())
        .expect("batch freeze modal must render the track N / M counter");
}

// ---------------------------------------------------------------------
// Gating + cancel — everything but Cancel is dropped mid-render.
// ---------------------------------------------------------------------

#[test]
fn transport_is_gated_while_freezing() {
    let mut app = build_arrange_app();
    set_freezing(&mut app, 0.5);
    // Route through the full gate-aware `update` entry point — the
    // pre-dispatch freeze gate is exactly what this test locks in.
    let _ = app.update(Message::Transport(TransportMessage::Play));
    assert!(
        !app.test_transport_playing(),
        "transport input must be gated while a freeze render is in flight"
    );
}

#[test]
fn cancel_clears_render_and_batch() {
    let mut app = build_arrange_app();
    start_freeze_all_batch(&mut app);
    assert!(app.test_freeze_queue().is_some());
    // Cancel is the gate's one whitelisted carve-out: it must flow through
    // the full `update` entry even while the render is in flight.
    let _ = app.update(Message::Freeze(FreezeMessage::CancelFreeze));
    assert!(
        app.test_freeze_queue().is_none(),
        "CancelFreeze must abandon the batch queue"
    );
    assert!(
        matches!(app.test_freeze_status(1), FreezeStatus::Idle),
        "CancelFreeze must roll the in-flight track back to idle"
    );
}

// ---------------------------------------------------------------------
// Golden — single freeze and batch run.
// ---------------------------------------------------------------------

#[test]
fn freeze_modal_single_golden() {
    let mut app = build_arrange_app();
    set_freezing(&mut app, 0.42);
    snapshot_to(&app, "tests/snapshots/freeze_progress_modal_single.png");
}

#[test]
fn freeze_modal_batch_golden() {
    let mut app = build_arrange_app();
    start_freeze_all_batch(&mut app);
    // Give the bar visible progress so the golden locks in a mid-render
    // frame (the queue counter stays "track 1 / M").
    if let Some(current) = app.test_freeze_queue().and_then(|q| q.current) {
        app.test_set_freeze_status(current, FreezeStatus::Freezing { fraction: 0.65 });
    }
    snapshot_to(&app, "tests/snapshots/freeze_progress_modal_batch.png");
}
