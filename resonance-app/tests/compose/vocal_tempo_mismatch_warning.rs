//! A section's vocal is rendered once, at its first placement's tempo, and
//! that one WAV plays at every placement (code review FU-M11b). Where a
//! placement sits in another tempo region, or the tempo changes inside a
//! placement, the vocal drifts against the grid; the vocal lane inspector
//! now says so instead of leaving it silent.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::compose::{ComposeMessage, SelectedLane};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{theme, Resonance};
use resonance_audio::types::TrackType;
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::methods::section as section_proto;
use resonance_control::{Request, Response};

use crate::common::roundtrip;

const VOCAL: u64 = 60;
const WINDOW: (f32, f32) = (1440.0, 1800.0);

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

fn ok(response: Response) {
    response.result::<serde_json::Value>().expect("control call succeeds");
}

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

/// Compose tab at a flat 100 BPM with a 4-bar section placed at bar 1 and
/// a vocal lane on it, selected in the inspector.
fn app_with_vocal_lane() -> (Resonance, SectionDefinitionId) {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_set_active_project(true);
    app.test_set_flat_tempo(100.0);
    app.test_add_track(VOCAL, TrackType::Vocal);
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
    call(
        &mut app,
        "section.set_lane_generator",
        &section_proto::SetLaneGeneratorParams {
            section_id,
            track_id: ProtoTrackId(VOCAL),
            kind: section_proto::LaneKind::Vocal,
            seed: None,
            options: None,
        },
    )
    .result::<section_proto::SetLaneGeneratorResult>()
    .expect("the vocal lane installs");
    app.test_dispatch(Message::Compose(ComposeMessage::SelectLane(
        SelectedLane::Instrument(VOCAL),
    )));
    (app, section_id)
}

fn warning(rendered: f32, other: f32) -> String {
    format!(
        "Sung at {rendered:.0} BPM (the first placement's tempo), but part of this \
         section plays at {other:.0} BPM \u{2014} the vocal drifts off the grid there."
    )
}

fn shows(app: &Resonance, text: &str) -> bool {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    ui.find(text).is_ok()
}

fn add_tempo(app: &mut Resonance, bar: u32, bpm: f32) {
    ok(call(
        app,
        "global.add_tempo_event",
        &serde_json::json!({ "bar": bar, "bpm": bpm }),
    ));
}

fn mismatch(app: &Resonance, section: SectionDefinitionId) -> Option<(f32, f32)> {
    app.test_vocal_tempo_mismatch(u64::from(section))
}

// Tempo points ramp linearly to the next point, so a tempo *region* is a
// pair of points one bar apart: the old tempo held, then the new one.

#[test]
fn one_tempo_everywhere_shows_no_warning() {
    let (mut app, section) = app_with_vocal_lane();
    assert!(shows(&app, "Lyrics"), "sanity: the vocal inspector is showing");
    assert_eq!(mismatch(&app, section), None);
    // A tempo change that ramps in only after the section's last bar.
    add_tempo(&mut app, 5, 100.0);
    add_tempo(&mut app, 6, 140.0);
    assert_eq!(mismatch(&app, section), None, "a change after the section is not inside it");
    assert!(!shows(&app, &warning(100.0, 140.0)));
}

#[test]
fn a_tempo_change_inside_the_section_warns() {
    let (mut app, section) = app_with_vocal_lane();
    // Ramps 100 -> 140 across bars 1-3, inside the 4-bar section.
    add_tempo(&mut app, 3, 140.0);
    assert_eq!(mismatch(&app, section), Some((100.0, 140.0)));
    assert!(shows(&app, &warning(100.0, 140.0)), "the inspector must show the warning");
}

#[test]
fn a_placement_in_another_tempo_region_warns() {
    let (mut app, section) = app_with_vocal_lane();
    add_tempo(&mut app, 16, 100.0);
    add_tempo(&mut app, 17, 140.0);
    assert_eq!(mismatch(&app, section), None, "one placement, one tempo");
    ok(call(
        &mut app,
        "section.place",
        &section_proto::PlaceParams {
            definition_id: section,
            start_bar: 21,
        },
    ));
    assert_eq!(mismatch(&app, section), Some((100.0, 140.0)));
    assert!(shows(&app, &warning(100.0, 140.0)));
}