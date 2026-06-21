//! Performance mode entry/exit (epic #11, todo #306).
//!
//! These exercise the real reducer (`update::ui`) through `Resonance::update`
//! to pin the toggle contract:
//!   * `F` (TogglePerformanceMode) enters Performance from the current view
//!     and toggles back to where it came from.
//!   * `Esc` (ExitPerformanceMode) only leaves Performance — a no-op elsewhere.
//!   * The Exit affordance / round-tripping restores the *previous* view, not
//!     a hard-coded Arrange.
//!   * Switching to/from Performance never disturbs transport state.
//!   * Record-arm never auto-opens Performance mode.

use resonance_app::message::{Message, UiMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, Resonance};

fn toggle(app: &mut Resonance) {
    let _ = app.update(Message::Ui(UiMessage::TogglePerformanceMode));
}

fn exit(app: &mut Resonance) {
    let _ = app.update(Message::Ui(UiMessage::ExitPerformanceMode));
}

fn switch(app: &mut Resonance, mode: ViewMode) {
    let _ = app.update(Message::Ui(UiMessage::SwitchView(mode)));
}

#[test]
fn f_toggles_into_and_back_out_of_performance() {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_view_mode(ViewMode::Compose);

    toggle(&mut app);
    assert_eq!(app.test_view_mode(), ViewMode::Performance);

    // Toggling again returns to the view we came from (Compose), not Arrange.
    toggle(&mut app);
    assert_eq!(app.test_view_mode(), ViewMode::Compose);
}

#[test]
fn esc_exits_performance_restoring_previous_view() {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_view_mode(ViewMode::Mixer);

    toggle(&mut app);
    assert_eq!(app.test_view_mode(), ViewMode::Performance);

    exit(&mut app);
    assert_eq!(app.test_view_mode(), ViewMode::Mixer);
}

#[test]
fn esc_is_a_noop_outside_performance() {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_view_mode(ViewMode::Arrange);

    exit(&mut app);
    assert_eq!(app.test_view_mode(), ViewMode::Arrange);

    app.test_set_view_mode(ViewMode::Compose);
    exit(&mut app);
    assert_eq!(app.test_view_mode(), ViewMode::Compose);
}

#[test]
fn switching_tabs_while_in_performance_clears_the_return_view() {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_view_mode(ViewMode::Compose);

    toggle(&mut app);
    assert_eq!(app.test_view_mode(), ViewMode::Performance);

    // User clicks the Mixer tab while in Performance. Now the remembered
    // "previous" view is gone; a later toggle into Performance should
    // remember Mixer, and Esc should fall back to Arrange only if nothing
    // was remembered.
    switch(&mut app, ViewMode::Mixer);
    assert_eq!(app.test_view_mode(), ViewMode::Mixer);

    toggle(&mut app);
    assert_eq!(app.test_view_mode(), ViewMode::Performance);
    exit(&mut app);
    assert_eq!(app.test_view_mode(), ViewMode::Mixer);
}

#[test]
fn entering_via_tab_button_then_esc_returns_to_source() {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_view_mode(ViewMode::Arrange);

    // The Performance tab button sends SwitchView(Performance) directly.
    switch(&mut app, ViewMode::Performance);
    assert_eq!(app.test_view_mode(), ViewMode::Performance);

    exit(&mut app);
    assert_eq!(app.test_view_mode(), ViewMode::Arrange);
}

#[test]
fn entering_and_leaving_preserves_transport_playing() {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_view_mode(ViewMode::Arrange);
    app.test_set_transport_playing(true);

    toggle(&mut app);
    assert_eq!(app.test_view_mode(), ViewMode::Performance);
    assert!(
        app.test_transport_playing(),
        "entering Performance must not stop playback"
    );

    exit(&mut app);
    assert_eq!(app.test_view_mode(), ViewMode::Arrange);
    assert!(
        app.test_transport_playing(),
        "leaving Performance must not stop playback"
    );
}

#[test]
fn arming_a_track_never_auto_opens_performance() {
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    app.test_set_view_mode(ViewMode::Arrange);

    // Arming the record flag must not, by itself, route into Performance.
    app.test_arm_first_track(true);
    let _ = app.update(Message::Tick);
    assert_eq!(
        app.test_view_mode(),
        ViewMode::Arrange,
        "record-arm must never auto-open Performance mode"
    );
}
