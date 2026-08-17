//! `DeviceParam` lanes on the Arrange automation overlay (ba todo #1094,
//! arch doc #162 §3, device definitions doc #201 §5).
//!
//! Regression guard for the bug where external-instrument device-param lanes
//! never drew in Arrange: `target_belongs_to_track` had no `DeviceParam` arm
//! (so the lane was never associated with its track), `target_priority` sent
//! `DeviceParam` to `u32::MAX`, and `target_label` returned an empty chip.
//!
//! Pure-fn coverage pins the track matching, the pick-order tier, the
//! deterministic tie-break, and the definition-resolved label; the golden
//! locks the rendered band + label chip on an external-instrument track.

use crate::common;

use std::collections::HashMap;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{
    AutomationMessage, ExternalInstrumentMessage as Eim, Message, UiMessage,
};
use resonance_app::state::{
    AutomationState, ExternalInstrumentMap, ExternalInstrumentState, TrackState, ViewMode,
};
use resonance_app::view::timeline::automation::{
    device_param_labels, primary_lane_for_track, target_belongs_to_track, target_label,
    target_priority,
};
use resonance_app::{theme, Resonance};
use resonance_audio::types::TrackId;
use resonance_common::{
    AutomationLane, AutomationTarget, CurveKind, DeviceDefinitionRegistry,
};

const TRACK: TrackId = 1;
/// The bundled Moog Muse preset ships in the registry, so it always resolves.
const MUSE: &str = "moog-muse";
const CUTOFF: &str = "filter1-cutoff";

fn device_target(track: TrackId, param_id: &str) -> AutomationTarget {
    AutomationTarget::DeviceParam {
        track,
        param_id: param_id.to_string(),
    }
}

/// An [`AutomationState`] holding one empty lane per target, ids ascending in
/// the given order.
fn automation_with(targets: &[AutomationTarget]) -> AutomationState {
    let mut state = AutomationState::default();
    for (i, target) in targets.iter().enumerate() {
        state.lanes.insert(
            target.clone(),
            AutomationLane::new(i as u64 + 1, target.clone(), Vec::new()),
        );
    }
    state
}

/// Registry with the bundled definitions plus an external-instrument map that
/// has the Muse preset selected on [`TRACK`] — the same resolution inputs the
/// mixer strip hands its picker.
fn muse_setup() -> (ExternalInstrumentMap, DeviceDefinitionRegistry) {
    let mut registry = DeviceDefinitionRegistry::default();
    registry.scan_bundled();
    let mut ext = ExternalInstrumentState::new(TRACK);
    ext.device_id = Some(MUSE.to_string());
    let mut map = ExternalInstrumentMap::new();
    map.insert(TRACK, ext);
    (map, registry)
}

// ---- Track matching (the `_ => false` fall-through bug) ----

#[test]
fn device_param_target_belongs_to_its_track() {
    let track = TrackState::new_instrument(TRACK, 0);
    assert!(
        target_belongs_to_track(&device_target(TRACK, CUTOFF), &track),
        "a DeviceParam lane must be associated with its own track"
    );
    assert!(
        !target_belongs_to_track(&device_target(TRACK + 1, CUTOFF), &track),
        "a DeviceParam lane on another track must not match"
    );
    // Bus/master targets still never belong to an arrange track row.
    assert!(!target_belongs_to_track(&AutomationTarget::BusGain(1), &track));
    assert!(!target_belongs_to_track(&AutomationTarget::MasterGain, &track));
}

// ---- Priority tier ----

#[test]
fn device_params_rank_after_mute_and_before_plugin_params() {
    let mute = target_priority(AutomationTarget::TrackMute(TRACK));
    let device = target_priority(device_target(TRACK, CUTOFF));
    let plugin = target_priority(AutomationTarget::PluginParam {
        instance: 9,
        param_id: 0,
    });
    // Same tier the mixer strip uses (view/mixer/automation.rs): gain, pan,
    // mute, then device params, then plugin params — the user automated the
    // device param deliberately, so it outranks generic plugin params.
    assert!(
        mute < device && device < plugin,
        "expected mute ({mute}) < device ({device}) < plugin param ({plugin})"
    );
    assert!(
        device < u32::MAX,
        "DeviceParam must no longer fall through to the u32::MAX arm"
    );
}

// ---- Primary-lane pick ----

#[test]
fn device_only_track_surfaces_a_primary_lane() {
    // The reported bug: an external-instrument track whose ONLY lanes are
    // DeviceParam lanes drew no automation band at all.
    let automation = automation_with(&[device_target(TRACK, CUTOFF)]);
    let track = TrackState::new_instrument(TRACK, 0);
    let lane = primary_lane_for_track(&automation, &track)
        .expect("a track with only DeviceParam lanes must still surface one");
    assert_eq!(lane.target, device_target(TRACK, CUTOFF));
}

#[test]
fn device_lane_ties_break_by_param_id() {
    let automation = automation_with(&[
        device_target(TRACK, "vcf-resonance"),
        device_target(TRACK, CUTOFF),
    ]);
    let track = TrackState::new_instrument(TRACK, 0);
    let lane = primary_lane_for_track(&automation, &track).expect("device lanes present");
    assert_eq!(
        lane.target,
        device_target(TRACK, CUTOFF),
        "ties within the device tier break by the lexicographically smallest param_id"
    );
}

#[test]
fn builtin_lanes_still_outrank_device_lanes() {
    let automation = automation_with(&[
        device_target(TRACK, CUTOFF),
        AutomationTarget::TrackGain(TRACK),
    ]);
    let track = TrackState::new_instrument(TRACK, 0);
    let lane = primary_lane_for_track(&automation, &track).expect("lanes present");
    assert_eq!(
        lane.target,
        AutomationTarget::TrackGain(TRACK),
        "gain keeps the top of the pick order"
    );
}

// ---- Label resolution ----

#[test]
fn labels_resolve_device_param_names_from_the_definition() {
    let (ext, registry) = muse_setup();
    let target = device_target(TRACK, CUTOFF);
    let automation = automation_with(&[target.clone()]);

    let labels = device_param_labels(&automation, &ext, &registry);
    let expected = registry
        .get(MUSE)
        .and_then(|def| def.param(CUTOFF))
        .map(|p| p.name.clone())
        .expect("the bundled Muse ships a filter1-cutoff param");
    assert_eq!(expected, "Filter 1 Cutoff");
    assert_eq!(labels.get(&target), Some(&expected));
    assert_eq!(
        target_label(&target, &labels),
        expected,
        "the band chip shows the definition's display name"
    );
}

#[test]
fn label_falls_back_to_the_raw_param_id_when_unresolved() {
    // No device selected → the resolution map is empty and the chip falls
    // back to the raw param id instead of an empty string.
    let target = device_target(TRACK, CUTOFF);
    let automation = automation_with(&[target.clone()]);
    let labels = device_param_labels(
        &automation,
        &ExternalInstrumentMap::new(),
        &DeviceDefinitionRegistry::default(),
    );
    assert!(labels.is_empty());
    assert_eq!(target_label(&target, &labels), CUTOFF);

    // Unknown param id on a resolving device → also absent → same fallback.
    let (ext, registry) = muse_setup();
    let ghost = device_target(TRACK, "no-such-param");
    let ghost_lanes = automation_with(&[ghost.clone()]);
    let labels = device_param_labels(&ghost_lanes, &ext, &registry);
    assert_eq!(target_label(&ghost, &labels), "no-such-param");

    // Non-device targets never consult the map.
    assert_eq!(
        target_label(&AutomationTarget::TrackGain(TRACK), &HashMap::new()),
        "Volume"
    );
}

// ---- Golden: the band + resolved label chip actually render ----

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

/// Arrange-pinned app with one external-instrument track (Muse selected) whose
/// only automation is a DeviceParam lane on the filter cutoff.
fn build_app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_view_mode(ViewMode::Arrange);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Arrange)));

    app.test_dispatch(Message::ExternalInstrument(Eim::Enable(TRACK)));
    app.test_dispatch(Message::ExternalInstrument(Eim::SetDevice(
        TRACK,
        Some(MUSE.to_string()),
    )));

    let target = device_target(TRACK, CUTOFF);
    app.test_dispatch(Message::Automation(AutomationMessage::AddLane(
        target.clone(),
    )));
    let sr = 48_000u64;
    for (frames, value) in [(0u64, 0.25f32), (sr, 0.9), (sr * 2, 0.5)] {
        app.test_dispatch(Message::Automation(AutomationMessage::AddBreakpoint {
            target: target.clone(),
            time_frames: frames,
            value,
            curve: CurveKind::Linear,
        }));
    }
    app
}

#[test]
fn timeline_device_param_lane_render() {
    let app = build_app();
    let mut ui = Simulator::with_size(sim_settings(), Size::new(1440.0, 900.0), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("arrange view with a DeviceParam lane should render");
    common::assert_golden(&snap, "tests/snapshots/timeline_device_param_lane.png");
}
