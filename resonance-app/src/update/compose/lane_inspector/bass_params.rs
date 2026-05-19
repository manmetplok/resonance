//! Handlers for `LaneInspectorMsg::SetBass*` — every Bass lane parameter
//! setter. Each function is a one-line wrapper around `update_lane_gen`
//! that pattern-matches the Bass generator and mutates one field.

use resonance_audio::types::TrackId;
use resonance_music_theory::{BassMotifMode, BassMotifPhrase, BassStyle};

use super::common::update_lane_gen;
use crate::compose::LaneGeneratorKind;

pub(super) fn set_style(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    style: BassStyle,
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Bass(p) = kind {
            p.style = style;
        }
    });
}

pub(super) fn set_base_note(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    note: u8,
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Bass(p) = kind {
            p.base_note = note;
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
        if let LaneGeneratorKind::Bass(p) = kind {
            p.velocity = v;
        }
    });
}

pub(super) fn set_motif_mode(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    mode: BassMotifMode,
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Bass(p) = kind {
            p.motif_mode = mode;
        }
    });
}

pub(super) fn set_motif_phrase(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    phrase: BassMotifPhrase,
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Bass(p) = kind {
            p.motif_phrase = phrase;
        }
    });
}
