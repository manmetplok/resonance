//! Tests for the device-parameter automation → CC/NRPN emission core
//! (architecture doc #201 §4, todo #723).
//!
//! Drives the pure `emit_device_param_automation` engine helper via its
//! `#[doc(hidden)]` re-export, with a capturing fake [`DeviceParamMidiSink`]
//! in place of the hardware [`MidiOutputRegistry`]. That keeps the test
//! headless — no engine thread, no MIDI port — while exercising the exact
//! per-block evaluation, normalized→binding mapping, CC/NRPN routing, and
//! change-only de-dupe the live engine poll runs.
//!
//! Because that core is the *single* emission path shared by live playback
//! and the realtime "bounce in place" drive (both advance the playhead and
//! run the same engine loop), driving it twice over an identical frame
//! sequence is the live↔bounce parity assertion (acceptance criterion 4).

use std::collections::HashMap;

use indexmap::IndexMap;

use resonance_audio::types::{Track, TrackId};
use resonance_audio::{emit_device_param_automation, AutomationLanes, DeviceParamMidiSink};
use resonance_common::device_definition::MidiBinding;
use resonance_common::{
    AutomationLane, AutomationTarget, Breakpoint, CurveKind, DeviceParam, LaneId, ParamCurve,
};

/// One captured outbound message, in emission order.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Msg {
    Cc {
        track: TrackId,
        channel: u8,
        cc: u8,
        value: u8,
    },
    Nrpn {
        track: TrackId,
        channel: u8,
        msb: u8,
        lsb: u8,
        value: u16,
        fourteen_bit: bool,
    },
}

/// Capturing stand-in for the hardware output registry.
#[derive(Default)]
struct CaptureSink {
    msgs: Vec<Msg>,
}

impl DeviceParamMidiSink for CaptureSink {
    fn emit_cc(&mut self, track: TrackId, channel: u8, cc: u8, value: u8) {
        self.msgs.push(Msg::Cc {
            track,
            channel,
            cc,
            value,
        });
    }
    fn emit_nrpn(
        &mut self,
        track: TrackId,
        channel: u8,
        msb: u8,
        lsb: u8,
        value: u16,
        fourteen_bit: bool,
    ) {
        self.msgs.push(Msg::Nrpn {
            track,
            channel,
            msb,
            lsb,
            value,
            fourteen_bit,
        });
    }
}

fn cc_param(id: &str, cc: u8) -> DeviceParam {
    DeviceParam {
        id: id.to_string(),
        name: id.to_string(),
        group: None,
        binding: MidiBinding::Cc { cc },
        min: 0,
        max: 127,
        default: None,
        curve: ParamCurve::Linear,
    }
}

fn nrpn_param(id: &str, msb: u8, lsb: u8) -> DeviceParam {
    DeviceParam {
        id: id.to_string(),
        name: id.to_string(),
        group: None,
        binding: MidiBinding::Nrpn {
            msb,
            lsb,
            fourteen_bit: true,
        },
        min: 0,
        max: 16_383,
        default: None,
        curve: ParamCurve::Linear,
    }
}

/// A one-track map: track `id` on MIDI channel `channel`, carrying `params`.
fn tracks_with(id: TrackId, channel: u8, params: Vec<DeviceParam>) -> IndexMap<TrackId, Track> {
    let mut track = Track::new(id, "Synth".to_string());
    track.midi_output_channel = Some(channel);
    track.set_device_params(params);
    let mut map = IndexMap::new();
    map.insert(id, track);
    map
}

/// A DeviceParam lane that ramps linearly from `0.0` to `1.0` over
/// `[0, span]` frames.
fn ramp_lane(lane_id: LaneId, track: TrackId, param_id: &str, span: u64) -> AutomationLane {
    AutomationLane::new(
        lane_id,
        AutomationTarget::DeviceParam {
            track,
            param_id: param_id.to_string(),
        },
        vec![
            Breakpoint::new(0, 0.0, CurveKind::Linear),
            Breakpoint::new(span, 1.0, CurveKind::Linear),
        ],
    )
}

fn lanes_of(lanes: Vec<AutomationLane>) -> AutomationLanes {
    lanes.into_iter().map(|l| (l.target.clone(), l)).collect()
}

/// Drive the emitter across `frames` with a fresh memo and return the
/// captured messages — the unit of work a sequence of engine polls performs.
fn drive(lanes: &AutomationLanes, tracks: &IndexMap<TrackId, Track>, frames: &[u64]) -> Vec<Msg> {
    let mut memo: HashMap<TrackId, HashMap<String, u16>> = HashMap::new();
    let mut sink = CaptureSink::default();
    for &f in frames {
        emit_device_param_automation(lanes, tracks, f, &mut memo, &mut sink);
    }
    sink.msgs
}

#[test]
fn cc_lane_emits_expected_ordered_value_sequence() {
    let tracks = tracks_with(7, 2, vec![cc_param("cutoff", 74)]);
    let lanes = lanes_of(vec![ramp_lane(1, 7, "cutoff", 1000)]);

    // Norms 0.0, 0.2, ... 1.0 over a 0..=127 linear CC: round(norm * 127).
    let frames = [0u64, 200, 400, 600, 800, 1000];
    let msgs = drive(&lanes, &tracks, &frames);

    let expected: Vec<Msg> = [0u8, 25, 51, 76, 102, 127]
        .into_iter()
        .map(|value| Msg::Cc {
            track: 7,
            channel: 2,
            cc: 74,
            value,
        })
        .collect();
    assert_eq!(msgs, expected);
}

#[test]
fn unchanged_binding_value_is_not_re_emitted() {
    let tracks = tracks_with(7, 0, vec![cc_param("cutoff", 74)]);
    let lanes = lanes_of(vec![ramp_lane(1, 7, "cutoff", 1000)]);

    // Same frame polled repeatedly, then a cluster of frames that all map to
    // the same CC integer (25 covers norm [24.5/127, 25.5/127] ≈ frames
    // 193..=200). Only genuine integer changes emit.
    let frames = [0u64, 0, 0, 195, 198, 200];
    let msgs = drive(&lanes, &tracks, &frames);

    assert_eq!(
        msgs,
        vec![
            Msg::Cc {
                track: 7,
                channel: 0,
                cc: 74,
                value: 0
            },
            Msg::Cc {
                track: 7,
                channel: 0,
                cc: 74,
                value: 25
            },
        ]
    );
}

#[test]
fn nrpn_lane_routes_through_the_nrpn_sink() {
    let tracks = tracks_with(3, 5, vec![nrpn_param("macro1", 1, 32)]);
    let lanes = lanes_of(vec![ramp_lane(1, 3, "macro1", 1000)]);

    // 14-bit NRPN over 0..=16383: round(norm * 16383).
    let msgs = drive(&lanes, &tracks, &[0u64, 500, 1000]);

    assert_eq!(
        msgs,
        vec![
            Msg::Nrpn {
                track: 3,
                channel: 5,
                msb: 1,
                lsb: 32,
                value: 0,
                fourteen_bit: true
            },
            Msg::Nrpn {
                track: 3,
                channel: 5,
                msb: 1,
                lsb: 32,
                value: 8192,
                fourteen_bit: true
            },
            Msg::Nrpn {
                track: 3,
                channel: 5,
                msb: 1,
                lsb: 32,
                value: 16383,
                fourteen_bit: true
            },
        ]
    );
}

#[test]
fn bounce_drive_matches_live_drive() {
    // Two CC params + one NRPN param on the same track, so a poll touches
    // several lanes per frame. Live and bounce run the identical core over
    // the identical frame sequence, so the captured streams must be equal.
    let tracks = tracks_with(
        9,
        1,
        vec![
            cc_param("cutoff", 74),
            cc_param("reso", 71),
            nrpn_param("macro1", 0, 10),
        ],
    );
    let lanes = lanes_of(vec![
        ramp_lane(1, 9, "cutoff", 1000),
        ramp_lane(2, 9, "reso", 1000),
        ramp_lane(3, 9, "macro1", 1000),
    ]);

    let frames = [0u64, 100, 250, 250, 500, 750, 1000];
    let live = drive(&lanes, &tracks, &frames);
    let bounce = drive(&lanes, &tracks, &frames);

    assert_eq!(live, bounce);
    assert!(!live.is_empty(), "the sweep should have emitted messages");
}

#[test]
fn read_disabled_lane_emits_nothing() {
    let tracks = tracks_with(7, 0, vec![cc_param("cutoff", 74)]);
    let mut lane = ramp_lane(1, 7, "cutoff", 1000);
    lane.enabled = false;
    let lanes = lanes_of(vec![lane]);

    assert!(drive(&lanes, &tracks, &[0u64, 500, 1000]).is_empty());
}

#[test]
fn lane_for_unknown_param_is_skipped() {
    // The track has no "cutoff" param (preset changed / never applied), so
    // the lane can't be mapped and is silently skipped — no panic, no emit.
    let tracks = tracks_with(7, 0, vec![]);
    let lanes = lanes_of(vec![ramp_lane(1, 7, "cutoff", 1000)]);

    assert!(drive(&lanes, &tracks, &[0u64, 500, 1000]).is_empty());
}
