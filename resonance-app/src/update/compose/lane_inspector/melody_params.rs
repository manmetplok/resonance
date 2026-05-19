//! Handlers for `LaneInspectorMsg::SetMelody*` / `ToggleMelody*` — every
//! Melody lane parameter setter. Each function pattern-matches the Melody
//! generator and mutates one field.

use resonance_audio::types::TrackId;
use resonance_music_theory::{ContourPreference, MelodyStyle};

use super::common::update_lane_gen;
use crate::compose::LaneGeneratorKind;

pub(super) fn set_style(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    style: MelodyStyle,
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Melody(p) = kind {
            p.style = style;
        }
    });
}

pub(super) fn set_register_low(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    note: u8,
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Melody(p) = kind {
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
        if let LaneGeneratorKind::Melody(p) = kind {
            p.register.1 = note;
        }
    });
}

pub(super) fn set_note_value(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    ticks: u32,
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Melody(p) = kind {
            p.note_value_ticks = ticks;
        }
    });
}

pub(super) fn set_rest_density(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    d: f32,
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Melody(p) = kind {
            p.rest_density = d;
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
        if let LaneGeneratorKind::Melody(p) = kind {
            p.velocity = v;
        }
    });
}

pub(super) fn set_articulation(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    a: f32,
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Melody(p) = kind {
            p.articulation = a;
        }
    });
}

pub(super) fn set_contour(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    contour: ContourPreference,
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Melody(p) = kind {
            p.contour = contour;
        }
    });
}

pub(super) fn set_phrase_len(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    len: u8,
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Melody(p) = kind {
            p.phrase_len = len;
        }
    });
}

pub(super) fn toggle_fill_vocal_gaps(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Melody(p) = kind {
            p.fill_vocal_gaps = !p.fill_vocal_gaps;
        }
    });
}
