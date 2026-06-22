//! Tests for the Performance-mode full-screen scaffold (ba todo #307,
//! epic #11, design doc #151).
//!
//! Two layers of coverage:
//!
//! 1. **Render smoke-tests** — the view layer returns Iced `Element`s
//!    that can't be asserted on structurally, so these exercise the
//!    happy-path that the scaffold builds without panicking across the
//!    transport states the status bar branches on (stopped / rehearsal
//!    / recording) and both with and without project content. A panic
//!    here (e.g. a bad slice/format on the telemetry labels) fails the
//!    test.
//! 2. **Golden-image snapshots** — the deliverable is an entirely
//!    visual surface (full-bleed chrome, telemetry clock, transport
//!    state cluster, Exit button, footer skeleton) that has to read
//!    correctly "from across the room" per #151, so the rendered
//!    chrome is locked against goldens under `tests/snapshots`. Four
//!    states are captured: empty/stopped, demo content stopped
//!    (telemetry populated), rehearsal (playing), and recording (BAD
//!    record chrome / REC dot).

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance};

/// Build the app, enter Performance mode, and render once. Returns the app
/// so callers can mutate state and render again.
fn enter_performance() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_view_mode(ViewMode::Performance);
    app
}

#[test]
fn renders_empty_project_scaffold() {
    let app = enter_performance();
    // No project content: the center stage shows its empty-state placeholder
    // and the telemetry reads the default transport. Must not panic.
    let _ = app.view();
}

#[test]
fn renders_with_demo_content() {
    let mut app = enter_performance();
    demo::seed_demo_content(&mut app);
    app.test_set_view_mode(ViewMode::Performance);
    let _ = app.view();
}

#[test]
fn renders_each_transport_state() {
    let mut app = enter_performance();

    // Stopped.
    app.test_set_transport_playing(false);
    app.test_set_transport_recording(false);
    let _ = app.view();

    // Rehearsal (playing, not recording).
    app.test_set_transport_playing(true);
    app.test_set_transport_recording(false);
    let _ = app.view();

    // Recording.
    app.test_set_transport_playing(true);
    app.test_set_transport_recording(true);
    let _ = app.view();
}

// -- Golden-image snapshots --------------------------------------------------

/// Window size matches the app's default & minimum window per the design
/// guidelines (and the other snapshot suites).
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// Build the iced simulator `Settings` with the same font registrations
/// the production app uses — without these the simulator falls back to a
/// default sans and the goldens stop matching the user's reality.
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

/// Render the Performance shell into the simulator at the standard window
/// size and assert it matches the golden at `path`.
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

/// Empty/stopped scaffold — no project content, so the center stage shows
/// its em-dash empty state and the telemetry reads the default transport.
#[test]
fn performance_scaffold_empty_stopped() {
    let app = enter_performance();
    snapshot_to(&app, "tests/snapshots/performance_scaffold_empty_stopped.png");
}

/// Demo content, transport stopped — telemetry clock / BPM / signature /
/// key are populated from the seeded project.
#[test]
fn performance_scaffold_demo_stopped() {
    let mut app = enter_performance();
    demo::seed_demo_content(&mut app);
    app.test_set_view_mode(ViewMode::Performance);
    app.test_set_transport_playing(false);
    app.test_set_transport_recording(false);
    snapshot_to(&app, "tests/snapshots/performance_scaffold_demo_stopped.png");
}

/// Rehearsal — demo content with the transport playing (not recording):
/// the centre cluster shows the play glyph + "REHEARSAL".
#[test]
fn performance_scaffold_rehearsal() {
    let mut app = enter_performance();
    demo::seed_demo_content(&mut app);
    app.test_set_view_mode(ViewMode::Performance);
    app.test_set_transport_playing(true);
    app.test_set_transport_recording(false);
    snapshot_to(&app, "tests/snapshots/performance_scaffold_rehearsal.png");
}

/// Recording — demo content with the transport recording: the centre
/// cluster shows the BAD-coloured REC dot + "RECORDING".
#[test]
fn performance_scaffold_recording() {
    let mut app = enter_performance();
    demo::seed_demo_content(&mut app);
    app.test_set_view_mode(ViewMode::Performance);
    app.test_set_transport_playing(true);
    app.test_set_transport_recording(true);
    snapshot_to(&app, "tests/snapshots/performance_scaffold_recording.png");
}
