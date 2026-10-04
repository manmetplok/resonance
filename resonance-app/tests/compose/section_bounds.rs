//! Section lengths and chord / placement positions are bounded, and the
//! overlap / fit invariants never overflow (code review VIEW-17).
//!
//! Any `u32` used to be accepted as a section length, so typing a huge
//! length in the New Section dialog hung the UI (the views loop over every
//! bar per frame). The invariants used plain `+`/`*`: a chord at
//! `start_beat = u32::MAX` panicked in debug and wrapped past the fit check
//! in release, and a placement near `u32::MAX` escaped the overlap check.

use resonance_app::compose::ComposeMessage;
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::{Request, Response, MAX_BARS};

use crate::common::roundtrip;

/// The GUI's section limit and the control API's promise are one number,
/// defined twice on purpose (domain vs wire, code review ARCH2-11).
#[test]
fn domain_section_limit_matches_the_wire_limit() {
    assert_eq!(resonance_app::compose::invariants::MAX_SECTION_BARS, MAX_BARS);
}

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_set_active_project(true);
    app
}

fn compose(app: &mut Resonance, msg: ComposeMessage) {
    let _ = app.update(Message::Compose(msg));
}

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

fn create(app: &mut Resonance, length_bars: u32) {
    compose(
        app,
        ComposeMessage::CreateSection {
            name: "Huge".to_owned(),
            length_bars,
            color: [1, 2, 3],
            place: true,
        },
    );
}

#[test]
fn a_section_longer_than_max_bars_is_rejected() {
    let mut app = app();
    create(&mut app, u32::MAX);
    assert!(app.compose_state().definitions.is_empty(), "u32::MAX-bar section created");
    assert!(app.compose_state().last_error.is_some());

    create(&mut app, MAX_BARS + 1);
    assert!(app.compose_state().definitions.is_empty(), "section past MAX_BARS created");
}

#[test]
fn the_new_section_dialog_rejects_a_huge_length() {
    let mut app = app();
    compose(&mut app, ComposeMessage::OpenCreateSectionDialog);
    compose(&mut app, ComposeMessage::SetNewSectionLength("100000000".to_owned()));
    compose(&mut app, ComposeMessage::ConfirmCreateSection);
    assert!(app.compose_state().definitions.is_empty(), "100000000-bar section created");
    assert!(app.compose_state().last_error.is_some());
}

#[test]
fn resizing_past_max_bars_is_rejected() {
    let mut app = app();
    create(&mut app, 4);
    let definition_id = app.compose_state().definitions[0].id;
    compose(
        &mut app,
        ComposeMessage::ResizeSection {
            definition_id,
            length_bars: u32::MAX,
        },
    );
    assert_eq!(app.compose_state().definitions[0].length_bars, 4);
    assert!(app.compose_state().last_error.is_some());
}

#[test]
fn a_placement_near_u32_max_is_rejected() {
    let mut app = app();
    create(&mut app, 4);
    let definition_id = app.compose_state().definitions[0].id;
    compose(
        &mut app,
        ComposeMessage::PlaceSection {
            definition_id,
            start_bar: u32::MAX - 1,
        },
    );
    assert_eq!(app.compose_state().placements.len(), 1, "a placement past MAX_BARS was added");
    assert!(app.compose_state().last_error.is_some());
}

#[test]
fn a_chord_at_u32_max_is_rejected_without_panicking() {
    let mut app = app();
    let section_id = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 4,
            scale: None,
            place: true,
        },
    )
    .result::<section_proto::CreateResult>()
    .expect("section.create succeeds")
    .section_id;
    let response = call(
        &mut app,
        "harmony.add_chord",
        &harmony_proto::AddChordParams {
            section_id,
            start_beat: f64::from(u32::MAX),
            duration_beats: 1.0,
            symbol: "Am".to_owned(),
        },
    );
    assert!(response.error.is_some(), "a chord at beat u32::MAX was accepted");
    let definition = &app.compose_state().definitions[0];
    assert!(definition.chords.is_empty());
}

/// FU-V2c: the same bounds hold for a project FILE — a hand-edited or
/// corrupt length must not reach the per-bar view loops unclamped, and a
/// placement may not end past the limit either.
#[test]
fn section_lengths_and_placements_from_a_file_are_clamped() {
    use resonance_app::compose::ComposeState;
    use resonance_app::project::{ProjectSectionDefinition, ProjectSectionPlacement};

    let parsed: Vec<ProjectSectionDefinition> = serde_json::from_value(serde_json::json!([
        {"id": 1, "name": "Huge", "color": [0, 0, 0], "length_bars": u32::MAX},
        {"id": 2, "name": "Empty", "color": [0, 0, 0], "length_bars": 0},
        {"id": 3, "name": "Late", "color": [0, 0, 0], "length_bars": 8},
    ]))
    .expect("definitions parse");
    let placements: Vec<ProjectSectionPlacement> = serde_json::from_value(serde_json::json!([
        {"id": 10, "definition_id": 3, "start_bar": MAX_BARS - 2},
    ]))
    .expect("placements parse");

    let mut state = ComposeState::default();
    state.load_from_project(&parsed, &placements);

    let lengths: Vec<u32> = state.definitions.iter().map(|d| d.length_bars).collect();
    assert_eq!(lengths, vec![MAX_BARS, 1, 8]);
    assert_eq!(
        state.placements[0].start_bar,
        MAX_BARS - 8,
        "the placement is pulled back to end at the limit"
    );
}
