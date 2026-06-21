//! Render smoke-tests for the Performance-mode full-screen scaffold
//! (ba todo #307, epic #11). The view layer returns Iced `Element`s that
//! can't be asserted on structurally, so these tests exercise the
//! happy-path that the scaffold builds without panicking across the
//! transport states the status bar branches on (stopped / rehearsal /
//! recording) and both with and without project content. A panic here
//! (e.g. a bad slice/format on the telemetry labels) fails the test.

use resonance_app::demo;
use resonance_app::state::ViewMode;
use resonance_app::Resonance;

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
