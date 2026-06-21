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

/// The resolution half of the `F` shortcut: `RequestPerformanceToggle` first
/// probes widget focus, then dispatches this with the result. `editing = true`
/// models "a text field was focused when `F` was pressed".
fn resolve_toggle(app: &mut Resonance, editing: bool) {
    let _ = app.update(Message::Ui(UiMessage::PerformanceToggleResolved { editing }));
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
fn typing_f_while_a_text_field_is_focused_does_not_toggle() {
    // Regression: the global `keyboard::listen()` subscription fires `F`
    // even while a text input is focused, so pressing `f` while editing a
    // track name / BPM / lyrics field must NOT flip Performance mode. The
    // focus probe resolves `editing = true`; the toggle is suppressed.
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_view_mode(ViewMode::Compose);

    resolve_toggle(&mut app, true);
    assert_eq!(
        app.test_view_mode(),
        ViewMode::Compose,
        "typing `f` into a focused text field must not enter Performance mode"
    );

    // It must equally not let the user *escape* Performance mid-edit.
    toggle(&mut app);
    assert_eq!(app.test_view_mode(), ViewMode::Performance);
    resolve_toggle(&mut app, true);
    assert_eq!(
        app.test_view_mode(),
        ViewMode::Performance,
        "typing `f` into a focused text field must not exit Performance mode"
    );
}

#[test]
fn pressing_f_with_no_text_field_focused_toggles() {
    // The complement of the regression test: when nothing is being edited
    // the focus probe resolves `editing = false` and `F` toggles as normal.
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_view_mode(ViewMode::Mixer);

    resolve_toggle(&mut app, false);
    assert_eq!(app.test_view_mode(), ViewMode::Performance);

    resolve_toggle(&mut app, false);
    assert_eq!(
        app.test_view_mode(),
        ViewMode::Mixer,
        "a second un-focused `f` returns to the source view"
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
