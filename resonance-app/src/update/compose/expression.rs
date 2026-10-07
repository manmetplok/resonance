//! Vocal expression-curve edits (vocal-mcp-control-and-quality.md §10).
//!
//! The curves live in each section's vocal lane config, section-relative;
//! the timeline edits them through the projected `Vocal:` automation rows
//! ([`crate::compose::expression_lanes`]). Every
//! [`AutomationMessage`] on an [`AutomationTarget::VocalCurve`] lands here
//! instead of in the engine-lane handlers: a frame is mapped to its
//! placement, then the section, then a beat offset, and the section's
//! curve is edited — so every placement of the section follows.
//!
//! The edit only changes the lane's params. Their fingerprint moves, so
//! `vocal_render_state::settle` marks the lane stale and, for a GUI edit,
//! arms the debounced re-render (§7); undo snapshots the params like any
//! other compose edit.

use iced::Task;

use resonance_audio::types::TrackId;
use resonance_common::{AutomationTarget, CurveKind as Shape};
use resonance_music_theory::{VocalCurvePoint, VocalParams};

use crate::compose::expression::{
    curve_audible, curve_mut, move_point, remove_point, set_stepped, upsert_point,
};
use crate::compose::expression_lanes::{
    plan_set_lane, project_vocal_lanes, vocal_params, ProjectionInputs, VocalLanes,
};
use crate::compose::vocal_svs::CurveKind;
use crate::compose::LaneGeneratorKind;
use crate::message::{AutomationMessage, Message};
use crate::Resonance;

/// The vocal curve rows as the compose model has them now.
pub(crate) fn vocal_lanes(r: &Resonance) -> VocalLanes {
    let bank = |params: &VocalParams| {
        let profile = r.voicebanks.profile(&params.voicebank);
        let mut support = [false; 4];
        for kind in CurveKind::ALL {
            support[kind.index()] = curve_audible(kind, &profile);
        }
        (support, profile.name())
    };
    project_vocal_lanes(&ProjectionInputs {
        definitions: &r.compose.definitions,
        placements: &r.compose.placements,
        tempo_map: &r.tempo_map,
        sample_rate: r.sample_rate,
        bank: &bank,
    })
}

/// Rebuild the projected rows the timeline draws from. Run after every
/// outermost update and after each curve edit.
pub(crate) fn refresh_vocal_lanes(r: &mut Resonance) {
    let lanes = vocal_lanes(r);
    if r.automation.vocal != lanes {
        r.automation.vocal = lanes;
    }
}

/// Apply an automation edit on a vocal curve row. `m.edited_target()` is a
/// `VocalCurve` target (the caller checked); anything else is a no-op.
pub(crate) fn handle(r: &mut Resonance, m: AutomationMessage) -> Task<Message> {
    let Some(target @ AutomationTarget::VocalCurve { track, curve }) = m.edited_target().cloned()
    else {
        return Task::none();
    };
    let lanes = vocal_lanes(r);
    let Some(lane) = lanes.get(&target) else {
        return Task::none();
    };
    match m {
        // The rows always exist and always play; there is no lane to add
        // and no Read flag.
        AutomationMessage::AddLane(_)
        | AutomationMessage::ToggleRead(_)
        | AutomationMessage::StartBreakpointDrag { .. } => {}
        AutomationMessage::RemoveLane(_) => {
            for window in &lane.windows {
                if let Some(points) = section_curve(r, window.definition_id, track, curve) {
                    points.clear();
                }
            }
        }
        AutomationMessage::AddBreakpoint {
            time_frames,
            value,
            curve: shape,
            ..
        } => {
            // Outside every placement, or on a bank that ignores the
            // curve, the row is inert.
            let Some(window) = lane.window_at(time_frames).filter(|w| w.supported) else {
                return Task::none();
            };
            let beat = window.beat_at(&r.tempo_map, r.sample_rate, time_frames);
            if let Some(points) = section_curve(r, window.definition_id, track, curve) {
                upsert_point(
                    points,
                    VocalCurvePoint {
                        beat,
                        value,
                        stepped: shape == Shape::Stepped,
                    },
                );
            }
        }
        AutomationMessage::DeleteBreakpoint { index, .. } => {
            if let Some(origin) = lane.origins.get(index) {
                if let Some(points) = section_curve(r, origin.definition_id, track, curve) {
                    remove_point(points, origin.index);
                }
            }
        }
        AutomationMessage::SetCurveKind {
            index, curve: shape, ..
        } => {
            if let Some(origin) = lane.origins.get(index) {
                if let Some(points) = section_curve(r, origin.definition_id, track, curve) {
                    set_stepped(points, origin.index, shape == Shape::Stepped);
                }
            }
        }
        AutomationMessage::DragBreakpoint {
            index,
            time_frames,
            value,
            ..
        } => {
            let Some(origin) = lane.origins.get(index) else {
                return Task::none();
            };
            let Some(window) = lane.window_of(origin.placement_id) else {
                return Task::none();
            };
            let beat = window.beat_at(&r.tempo_map, r.sample_rate, time_frames);
            let len = window.len_beats;
            if let Some(points) = section_curve(r, origin.definition_id, track, curve) {
                move_point(points, origin.index, beat, value, len);
            }
        }
        AutomationMessage::SetLane { points, .. } => {
            let current = |def: u64| {
                r.compose
                    .find_definition(def)
                    .and_then(|d| vocal_params(d, track))
                    .map(|p| crate::compose::expression::curve(&p.curves, curve).to_vec())
                    .unwrap_or_default()
            };
            // The control layer validated the same plan before sending;
            // a plan that fails here changes nothing.
            let Ok(plan) = plan_set_lane(lane, &points, &current, &r.tempo_map, r.sample_rate)
            else {
                return Task::none();
            };
            for (def, new_curve) in plan {
                if let Some(points) = section_curve(r, def, track, curve) {
                    *points = new_curve;
                }
            }
        }
        AutomationMessage::EndBreakpointDrag
        | AutomationMessage::CycleTrackLane(_)
        | AutomationMessage::ToggleTrackExpanded(_) => {}
    }
    refresh_vocal_lanes(r);
    Task::none()
}

/// The `curve` points of `track`'s vocal lane in `definition`, for editing.
fn section_curve(
    r: &mut Resonance,
    definition: u64,
    track: TrackId,
    curve: CurveKind,
) -> Option<&mut Vec<VocalCurvePoint>> {
    let def = r.compose.definitions.iter_mut().find(|d| d.id == definition)?;
    match &mut def.lane_generators.get_mut(&track)?.kind {
        LaneGeneratorKind::Vocal(params) => Some(curve_mut(&mut params.curves, curve)),
        _ => None,
    }
}
