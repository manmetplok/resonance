//! The vocal expression-curve target `{vocal: {track_id, curve}}`
//! (vocal-mcp-control-and-quality.md §10).
//!
//! A vocal lane is the projection of each section's curve onto every
//! placement of the section ([`crate::compose::expression_lanes`]); the
//! generic `automation.*` handlers read and write it like any lane, with
//! song-position points. What differs lives here: resolving the target,
//! refusing a write the section mapping cannot take (a point outside every
//! placement, two values for one section beat) before anything is
//! dispatched, and `song.vocal`'s per-lane curve list.

use super::target::ResolvedTarget;
use super::value::ValueDomain;
use crate::compose::expression::{curve, curve_audible};
use crate::compose::expression_lanes::{plan_set_lane, vocal_params, PlanError};
use crate::compose::vocal_svs::{BankProfile, CurveKind};
use crate::update::control::plugin_target::ChainOwner;
use crate::update::control::reply::no_track;
use crate::update::control::view_model;
use crate::Resonance;
use resonance_common::{AutomationTarget, Breakpoint};
use resonance_control::methods::automation::{AutomationTargetSpec, VocalCurve, VocalCurveTarget};
use resonance_control::methods::song::VocalCurveSummary;
use resonance_control::RpcError;
use resonance_music_theory::VocalParams;

/// The model's curve kind for the wire's.
pub(in crate::update::control) fn model_kind(curve: VocalCurve) -> CurveKind {
    match curve {
        VocalCurve::Dynamics => CurveKind::Dynamics,
        VocalCurve::Tension => CurveKind::Tension,
        VocalCurve::Breathiness => CurveKind::Breathiness,
        VocalCurve::PitchBend => CurveKind::PitchBend,
    }
}

/// The wire's curve kind for the model's.
pub(in crate::update::control) fn wire_kind(kind: CurveKind) -> VocalCurve {
    match kind {
        CurveKind::Dynamics => VocalCurve::Dynamics,
        CurveKind::Tension => VocalCurve::Tension,
        CurveKind::Breathiness => VocalCurve::Breathiness,
        CurveKind::PitchBend => VocalCurve::PitchBend,
    }
}

/// The target spec that addresses `track`'s `kind` curve.
pub(in crate::update::control) fn spec_for(track: u64, kind: CurveKind) -> AutomationTargetSpec {
    AutomationTargetSpec {
        vocal: Some(VocalCurveTarget {
            track_id: resonance_control::ids::TrackId(track),
            curve: wire_kind(kind),
        }),
        ..AutomationTargetSpec::default()
    }
}

/// Resolve `{vocal: …}`: no other target field beside it, an existing
/// track, and a vocal lane on it in at least one section.
pub(in crate::update::control) fn resolve(
    app: &Resonance,
    spec: &AutomationTargetSpec,
    vocal: &VocalCurveTarget,
) -> Result<ResolvedTarget, RpcError> {
    let others = spec.track_id.is_some()
        || spec.bus_id.is_some()
        || spec.master
        || spec.control.is_some()
        || spec.param.is_some()
        || spec.plugin_id.is_some()
        || spec.occurrence.is_some();
    if others {
        return Err(RpcError::invalid_params(
            "a vocal curve lane is addressed by vocal: {track_id, curve} alone; drop track_id / \
             bus_id / master / control / param / plugin_id / occurrence",
        ));
    }
    let track = vocal.track_id.0;
    if !app.registry.tracks.iter().any(|t| t.id == track) {
        return Err(no_track(track));
    }
    let has_lane = app
        .compose
        .definitions
        .iter()
        .any(|d| vocal_params(d, track).is_some());
    if !has_lane {
        return Err(RpcError::invalid_params(format!(
            "track {track} has no vocal lane in any section; give a section a Vocal lane \
             generator for it first (section_set_lane_generator)"
        )));
    }
    let curve = model_kind(vocal.curve);
    Ok(ResolvedTarget {
        target: AutomationTarget::VocalCurve { track, curve },
        domain: ValueDomain::Vocal(curve),
        owner: ChainOwner::Track(track),
        plugin: None,
    })
}

/// Refuse a whole-lane replace of a vocal lane the sections cannot hold:
/// a new point outside every placement, two different values for one beat
/// of one section, or switching the lane off. Runs the same plan the
/// update handler applies, so a write that passes here applies whole.
pub(in crate::update::control) fn check_set_lane(
    app: &Resonance,
    target: &AutomationTarget,
    points: &[Breakpoint],
    enabled: Option<bool>,
) -> Result<(), RpcError> {
    let AutomationTarget::VocalCurve { track, curve: kind } = target else {
        return Ok(());
    };
    if enabled == Some(false) {
        return Err(RpcError::invalid_params(
            "a vocal curve lane always plays; it has no Read flag to turn off",
        ));
    }
    let Some(lane) = app.automation.vocal.get(target) else {
        return Err(RpcError::invalid_params(format!(
            "track {track} has no vocal lane in any section"
        )));
    };
    let current = |def: u64| {
        app.compose
            .find_definition(def)
            .and_then(|d| vocal_params(d, *track))
            .map(|p| curve(&p.curves, *kind).to_vec())
            .unwrap_or_default()
    };
    match plan_set_lane(lane, points, &current, &app.tempo_map, app.sample_rate) {
        Ok(_) => Ok(()),
        Err(PlanError::OutsidePlacements { frame }) => {
            let at = view_model::song_position(app, frame);
            Err(RpcError::invalid_params(format!(
                "a point at bar {} beat {} lies outside every placement of a section with a \
                 vocal lane on track {track}; vocal curves are stored per section, so a point \
                 must fall inside one (song_sections lists the placements)",
                at.bar, at.beat
            )))
        }
        Err(PlanError::Conflict { first, second }) => {
            let (a, b) = (
                view_model::song_position(app, first),
                view_model::song_position(app, second),
            );
            Err(RpcError::invalid_params(format!(
                "points at bar {} beat {} and bar {} beat {} are the same beat of one section \
                 (two placements of it) with different values; a section has one curve, so \
                 give one of them",
                a.bar, a.beat, b.bar, b.beat
            )))
        }
    }
}

/// `song.vocal`'s per-lane curve list: the curves `params` has points on.
pub(in crate::update::control) fn curve_summaries(
    params: &VocalParams,
    bank: &BankProfile,
) -> Vec<VocalCurveSummary> {
    CurveKind::ALL
        .iter()
        .filter_map(|&kind| {
            let points = curve(&params.curves, kind).len();
            (points > 0).then(|| VocalCurveSummary {
                curve: wire_kind(kind),
                points: points as u32,
                supported: curve_audible(kind, bank),
            })
        })
        .collect()
}
