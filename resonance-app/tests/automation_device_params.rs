//! Automation parameter picker → device-param lanes (epic #40, doc #201 §5,
//! ba todo #726 / A3).
//!
//! For an external-instrument track with a device preset selected, the mixer
//! strip's automation "pick parameter" list gains the device definition's
//! named [`DeviceParam`]s, clustered by group, each producing an
//! `AutomationTarget::DeviceParam { track, param_id }` lane. The picker hides
//! device params entirely when no preset is selected. Lane create / Read
//! toggle / breakpoint editing reuse the existing automation paths unchanged,
//! so this file focuses on the *picker* content and the resulting lane target.
//!
//! A closed `pick_list` renders only its placeholder, so the picker option
//! list is asserted through the deterministic `test_automation_picker_labels`
//! accessor (mirrors `choices_for`) — the GPU-independent companion to the
//! golden snapshot, matching the epic-#14 picker's own test strategy.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{
    AutomationMessage, ExternalInstrumentMessage as Eim, Message, UiMessage,
};
use resonance_app::state::{TrackState, ViewMode};
use resonance_app::{theme, Resonance, STARTUP_TAB};
use resonance_audio::types::TrackId;
use resonance_common::AutomationTarget;

const TRACK: TrackId = 1;
/// The bundled Moog Muse preset ships in the registry, so `Resonance::new()`
/// always resolves it.
const MUSE: &str = "moog-muse";

/// App with an active project and a single external-instrument track. The
/// device preset is left unselected; individual tests select it.
fn external_app() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    app.test_dispatch(Message::ExternalInstrument(Eim::Enable(TRACK)));
    app
}

fn select_muse(app: &mut Resonance) {
    app.test_dispatch(Message::ExternalInstrument(Eim::SetDevice(
        TRACK,
        Some(MUSE.to_string()),
    )));
}

/// The always-present built-in targets a track strip offers, regardless of
/// device selection.
fn has_builtins(labels: &[String]) {
    for base in ["Volume", "Pan", "Mute"] {
        assert!(
            labels.iter().any(|l| l == base),
            "built-in target {base:?} must always be offered; got {labels:?}"
        );
    }
}

#[test]
fn picker_hides_device_params_without_a_preset() {
    let app = external_app();
    let labels = app.test_automation_picker_labels(TRACK);

    has_builtins(&labels);
    // No preset selected ⇒ exactly the three built-ins, no device params.
    assert_eq!(
        labels.len(),
        3,
        "with no device preset the picker shows only gain/pan/mute; got {labels:?}"
    );
    assert!(
        !labels.iter().any(|l| l.contains("Filter")),
        "device params must be hidden until a preset is selected"
    );
}

#[test]
fn picker_exposes_named_device_params_when_preset_selected() {
    let mut app = external_app();
    select_muse(&mut app);
    let labels = app.test_automation_picker_labels(TRACK);

    has_builtins(&labels);
    // The bundled Muse ships 102 named params on top of the 3 built-ins.
    assert!(
        labels.len() > 3,
        "selecting a preset must add the device's named params; got {} labels",
        labels.len()
    );
    // Named params appear, prefixed by their group so related controls
    // cluster in the dropdown ("Filter: Filter 1 Cutoff").
    assert!(
        labels.iter().any(|l| l == "Filter: Filter 1 Cutoff"),
        "the picker must list the Muse's grouped named params; got {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l == "Amp Envelope: VCA Env Attack"),
        "params from every group are offered; got {labels:?}"
    );
}

#[test]
fn device_params_are_clustered_by_group() {
    let mut app = external_app();
    select_muse(&mut app);
    let labels = app.test_automation_picker_labels(TRACK);

    // Every "Filter: …" label must be contiguous — clustering by group means
    // no non-Filter label sits between two Filter labels.
    let filter_positions: Vec<usize> = labels
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("Filter: "))
        .map(|(i, _)| i)
        .collect();
    assert!(
        filter_positions.len() >= 2,
        "expected several Filter params; got {filter_positions:?}"
    );
    let (first, last) = (
        *filter_positions.first().unwrap(),
        *filter_positions.last().unwrap(),
    );
    assert_eq!(
        last - first + 1,
        filter_positions.len(),
        "Filter-group params must be contiguous (clustered); labels: {labels:?}"
    );
}

#[test]
fn picker_hides_device_params_again_after_clearing_the_preset() {
    let mut app = external_app();
    select_muse(&mut app);
    assert!(app.test_automation_picker_labels(TRACK).len() > 3);

    // Clear the selection — device params vanish, built-ins remain.
    app.test_dispatch(Message::ExternalInstrument(Eim::SetDevice(TRACK, None)));
    let labels = app.test_automation_picker_labels(TRACK);
    has_builtins(&labels);
    assert_eq!(
        labels.len(),
        3,
        "clearing the preset must hide device params again; got {labels:?}"
    );
}

/// Adding a device-param lane reuses the existing automation update path and
/// produces an `AutomationTarget::DeviceParam { track, param_id }` lane keyed
/// to the track — the same lane the picker's selection dispatches.
#[test]
fn adding_a_device_param_lane_creates_a_deviceparam_target() {
    let mut app = external_app();
    select_muse(&mut app);

    let target = AutomationTarget::DeviceParam {
        track: TRACK,
        param_id: "filter1-cutoff".to_string(),
    };
    app.test_dispatch(Message::Automation(AutomationMessage::AddLane(
        target.clone(),
    )));

    let lanes = &app.test_automation().lanes;
    assert!(
        lanes.contains_key(&target),
        "AddLane on a device param creates a DeviceParam lane; lanes: {:?}",
        lanes.keys().collect::<Vec<_>>()
    );
}

/// End-to-end render guard: a `DeviceParam` lane on an external-instrument
/// track surfaces the strip's automation lane header (Read toggle) through the
/// existing `automation_header` path — proving device-param lanes reuse the
/// mixer's automation controls unchanged. Mirrors the epic-#14 picker's
/// simulator render check (golden PNGs diverge in this environment, so this is
/// the deterministic stand-in).
#[test]
fn mixer_renders_device_param_lane_header() {
    let _ = STARTUP_TAB.set(ViewMode::Mixer);
    let mut app = external_app();
    select_muse(&mut app);
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));

    // No lane yet ⇒ no Read toggle on any strip.
    {
        let mut ui = simulator(&app);
        assert!(
            ui.find("READ").is_err(),
            "no Read toggle should render before a device-param lane exists"
        );
    }

    // Point a lane at the Muse's filter cutoff.
    app.test_dispatch(Message::Automation(AutomationMessage::AddLane(
        AutomationTarget::DeviceParam {
            track: TRACK,
            param_id: "filter1-cutoff".to_string(),
        },
    )));

    let mut ui = simulator(&app);
    ui.find("READ")
        .expect("the device-param lane's header (Read toggle) should render on the track strip");
}

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = vec![theme::ICON_FONT_BYTES.into()];
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    let settings = iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    };
    Simulator::with_size(settings, Size::new(1440.0, 900.0), app.view())
}
