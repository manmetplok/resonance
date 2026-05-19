//! Handlers for `LaneInspectorMsg::SetPad*` — every Pad lane parameter
//! setter. Each function pattern-matches the Pad generator and mutates one
//! field.

use resonance_audio::types::TrackId;

use super::common::update_lane_gen;
use crate::compose::LaneGeneratorKind;

pub(super) fn set_register_low(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    note: u8,
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Pad(p) = kind {
            p.register.0 = note;
        }
    });
}

pub(super) fn set_register_high(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    note: u8,
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Pad(p) = kind {
            p.register.1 = note;
        }
    });
}

pub(super) fn set_velocity(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    v: f32,
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Pad(p) = kind {
            p.velocity = v;
        }
    });
}
