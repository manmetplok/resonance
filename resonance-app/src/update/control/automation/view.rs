//! Lane → wire projection (automation-control-api.md §4.3, §4.5).
//!
//! Every lane view — `automation.lanes`, the `{revision, lane}` every
//! write replies with, the compact lists in `song.tracks` /
//! `master.summary` and the `automation_lanes` counts — comes out of
//! [`LaneContext::describe`], so they cannot disagree about a lane's
//! owner, name or status.

use super::value::ValueDomain;
use crate::update::control::plugin_target::ChainOwner;
use crate::update::control::view_model;
use crate::Resonance;
use resonance_audio::types::PluginInstanceId;
use resonance_common::{AutomationLane, AutomationTarget, CurveKind};
use resonance_control::ids::TrackId;
use resonance_control::methods::automation::{
    AutomationCurve, AutomationTargetSpec, LaneControl, LaneStatus, LaneSummary, LaneTargetView,
    LaneView, PointView,
};
use resonance_control::methods::track::{PluginParamsEntry, PluginSlotStatus};
use std::collections::HashMap;

/// Everything a lane's description needs that is shared by all lanes of
/// one read: where each plugin instance sits, and the mixer order of the
/// owners. Built once per call so listing N lanes is not N chain scans.
pub(in crate::update::control) struct LaneContext {
    plugins: HashMap<PluginInstanceId, (ChainOwner, PluginParamsEntry)>,
    track_order: HashMap<u64, usize>,
    bus_order: HashMap<u64, usize>,
}

/// One lane, described: the pieces every projection is cut from.
#[derive(Debug, Clone)]
pub(in crate::update::control) struct LaneInfo {
    /// `None` for an orphaned plugin lane — its instance is gone.
    pub owner: Option<ChainOwner>,
    pub target: LaneTargetView,
    pub status: LaneStatus,
    pub domain: ValueDomain,
    sort_key: (u8, usize, u32, u32, u32, String),
}

impl LaneContext {
    pub(in crate::update::control) fn new(app: &Resonance) -> Self {
        let mut plugins = HashMap::new();
        for t in app.sorted_tracks() {
            for (slot, entry) in view_model::plugin_entries(app, t).into_iter().enumerate() {
                if let Some(p) = t.plugins.get(slot) {
                    plugins.insert(p.instance_id, (ChainOwner::Track(t.id), entry));
                }
            }
        }
        for b in app.sorted_busses() {
            for (slot, entry) in view_model::bus_plugin_entries(b).into_iter().enumerate() {
                if let Some(p) = b.plugins.get(slot) {
                    plugins.insert(p.instance_id, (ChainOwner::Bus(b.id), entry));
                }
            }
        }
        for (slot, entry) in view_model::master_plugin_entries(app)
            .into_iter()
            .enumerate()
        {
            if let Some(p) = app.master.plugins.get(slot) {
                plugins.insert(p.instance_id, (ChainOwner::Master, entry));
            }
        }
        Self {
            plugins,
            track_order: app
                .sorted_tracks()
                .iter()
                .enumerate()
                .map(|(i, t)| (t.id, i))
                .collect(),
            bus_order: app
                .sorted_busses()
                .iter()
                .enumerate()
                .map(|(i, b)| (b.id, i))
                .collect(),
        }
    }

    /// Describe one lane.
    pub(in crate::update::control) fn describe(&self, lane: &AutomationLane) -> LaneInfo {
        let target = &lane.target;
        let mixer = |owner: ChainOwner, control: LaneControl, exists: bool| {
            let rank = match control {
                LaneControl::Volume => 0,
                LaneControl::Pan => 1,
                LaneControl::Mute => 2,
                LaneControl::Device => 5,
            };
            LaneInfo {
                owner: Some(owner),
                target: LaneTargetView {
                    spec: AutomationTargetSpec {
                        control: Some(control),
                        ..owner_spec(owner)
                    },
                    plugin_name: None,
                    param_id: None,
                    param_name: None,
                },
                status: if exists {
                    LaneStatus::Ok
                } else {
                    LaneStatus::Orphaned
                },
                domain: ValueDomain::for_mixer(target),
                sort_key: self.sort_key(Some(owner), rank, 0, 0),
            }
        };
        match target {
            AutomationTarget::TrackGain(id) => mixer(
                ChainOwner::Track(*id),
                LaneControl::Volume,
                self.track_order.contains_key(id),
            ),
            AutomationTarget::TrackPan(id) => mixer(
                ChainOwner::Track(*id),
                LaneControl::Pan,
                self.track_order.contains_key(id),
            ),
            AutomationTarget::TrackMute(id) => mixer(
                ChainOwner::Track(*id),
                LaneControl::Mute,
                self.track_order.contains_key(id),
            ),
            AutomationTarget::BusGain(id) => mixer(
                ChainOwner::Bus(*id),
                LaneControl::Volume,
                self.bus_order.contains_key(id),
            ),
            AutomationTarget::BusPan(id) => mixer(
                ChainOwner::Bus(*id),
                LaneControl::Pan,
                self.bus_order.contains_key(id),
            ),
            AutomationTarget::BusMute(id) => mixer(
                ChainOwner::Bus(*id),
                LaneControl::Mute,
                self.bus_order.contains_key(id),
            ),
            AutomationTarget::MasterGain => mixer(ChainOwner::Master, LaneControl::Volume, true),
            AutomationTarget::DeviceParam { track, param_id } => {
                let mut info = mixer(
                    ChainOwner::Track(*track),
                    LaneControl::Device,
                    self.track_order.contains_key(track),
                );
                info.target.spec.param = Some(param_id.clone());
                info.sort_key.5 = param_id.clone();
                info
            }
            AutomationTarget::PluginParam { instance, param_id } => {
                self.describe_plugin_lane(*instance, *param_id)
            }
        }
    }

    fn describe_plugin_lane(&self, instance: PluginInstanceId, param_id: u32) -> LaneInfo {
        let Some((owner, entry)) = self.plugins.get(&instance) else {
            // The instance is gone: nothing is known but the param id.
            return LaneInfo {
                owner: None,
                target: LaneTargetView {
                    spec: AutomationTargetSpec {
                        param: Some(param_id.to_string()),
                        ..AutomationTargetSpec::default()
                    },
                    plugin_name: None,
                    param_id: Some(param_id),
                    param_name: None,
                },
                status: LaneStatus::Orphaned,
                domain: ValueDomain::Normalized,
                sort_key: self.sort_key(None, 10, 0, param_id),
            };
        };
        let param = entry.params.iter().find(|p| p.id == param_id);
        let status = if entry.status == PluginSlotStatus::Missing {
            LaneStatus::PluginMissing
        } else if entry.params.is_empty() {
            LaneStatus::PluginInitializing
        } else if param.is_none() {
            LaneStatus::Orphaned
        } else {
            LaneStatus::Ok
        };
        // Report the name when it resolves back to THIS parameter (a
        // plugin may reuse a display name across modules), else the id —
        // so the target can always be passed straight back.
        let param_str = match param {
            Some(p)
                if crate::update::control::track::find_param(&entry.params, &p.name)
                    .is_some_and(|found| found.id == p.id) =>
            {
                p.name.clone()
            }
            _ => param_id.to_string(),
        };
        LaneInfo {
            owner: Some(*owner),
            target: LaneTargetView {
                spec: AutomationTargetSpec {
                    param: Some(param_str),
                    plugin_id: Some(entry.plugin_id.clone()),
                    occurrence: Some(entry.occurrence),
                    ..owner_spec(*owner)
                },
                plugin_name: Some(entry.name.clone()),
                param_id: Some(param_id),
                param_name: param.map(|p| p.name.clone()),
            },
            status,
            domain: match param {
                Some(p) => ValueDomain::Plugin(p.clone()),
                None => ValueDomain::Normalized,
            },
            sort_key: self.sort_key(Some(*owner), 10, entry.slot, param_id),
        }
    }

    /// `(owner group, owner position, kind rank, slot, param id, device
    /// param)`; [`sorted_lanes`] breaks the remaining ties by lane id.
    fn sort_key(
        &self,
        owner: Option<ChainOwner>,
        rank: u32,
        slot: u32,
        param_id: u32,
    ) -> (u8, usize, u32, u32, u32, String) {
        let (group, pos) = match owner {
            Some(ChainOwner::Track(id)) => {
                (0, self.track_order.get(&id).copied().unwrap_or(usize::MAX))
            }
            Some(ChainOwner::Bus(id)) => {
                (1, self.bus_order.get(&id).copied().unwrap_or(usize::MAX))
            }
            Some(ChainOwner::Master) => (2, 0),
            None => (3, 0),
        };
        (group, pos, rank, slot, param_id, String::new())
    }
}

/// The owner fields of a target spec.
fn owner_spec(owner: ChainOwner) -> AutomationTargetSpec {
    match owner {
        ChainOwner::Track(id) => AutomationTargetSpec {
            track_id: Some(TrackId(id)),
            ..AutomationTargetSpec::default()
        },
        ChainOwner::Bus(id) => AutomationTargetSpec {
            bus_id: Some(TrackId(id)),
            ..AutomationTargetSpec::default()
        },
        ChainOwner::Master => AutomationTargetSpec {
            master: true,
            ..AutomationTargetSpec::default()
        },
    }
}

/// Every lane in the project, described, in listing order: tracks in
/// mixer order, then busses, then master, then orphans; within an owner
/// volume, pan, mute, device lanes, then plugin lanes by slot and param.
pub(in crate::update::control) fn sorted_lanes<'a>(
    app: &'a Resonance,
    ctx: &LaneContext,
) -> Vec<(LaneInfo, &'a AutomationLane)> {
    let mut lanes: Vec<(LaneInfo, &AutomationLane)> = app
        .automation
        .lanes
        .values()
        .map(|lane| (ctx.describe(lane), lane))
        .collect();
    lanes.sort_by(|(a, la), (b, lb)| a.sort_key.cmp(&b.sort_key).then(la.id.cmp(&lb.id)));
    lanes
}

/// The full view of `lane`, its points windowed to `[start, end)` when
/// `window` is given (each point keeps its whole-lane index).
pub(in crate::update::control) fn lane_view(
    app: &Resonance,
    info: &LaneInfo,
    lane: &AutomationLane,
    window: Option<(u64, u64)>,
) -> LaneView {
    let points = lane
        .points
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            window.is_none_or(|(start, end)| p.time_frames >= start && p.time_frames < end)
        })
        .map(|(index, p)| {
            let (value, text) = info.domain.real(&lane.target, p.value);
            PointView {
                index: index as u32,
                position: view_model::song_position(app, p.time_frames),
                value,
                normalized: f64::from(p.value),
                text,
                curve: wire_curve(p.curve),
            }
        })
        .collect();
    LaneView {
        lane_id: lane.id,
        target: info.target.clone(),
        enabled: lane.enabled,
        status: info.status,
        unit: info.domain.unit(),
        min: info.domain.min(),
        max: info.domain.max(),
        point_count: lane.points.len() as u32,
        points,
    }
}

/// The lane on `target` as it reads back now, or `None` when there is no
/// such lane — what every write replies with.
pub(in crate::update::control) fn lane_view_for(
    app: &Resonance,
    target: &AutomationTarget,
) -> Option<LaneView> {
    let lane = app.automation.lanes.get(target)?;
    let ctx = LaneContext::new(app);
    Some(lane_view(app, &ctx.describe(lane), lane, None))
}

/// The compact summaries of every lane `owner` carries, in listing order.
pub(in crate::update::control) fn lane_summaries(
    app: &Resonance,
    owner: ChainOwner,
) -> Vec<LaneSummary> {
    if app.automation.lanes.is_empty() {
        return Vec::new();
    }
    let ctx = LaneContext::new(app);
    sorted_lanes(app, &ctx)
        .into_iter()
        .filter(|(info, _)| info.owner == Some(owner))
        .map(|(info, lane)| LaneSummary {
            lane_id: lane.id,
            control: info.target.spec.control,
            param: info.target.spec.param.clone(),
            plugin_id: info.target.spec.plugin_id.clone(),
            occurrence: info.target.spec.occurrence,
            points: lane.points.len() as u32,
            enabled: lane.enabled,
            status: info.status,
        })
        .collect()
}

/// How many lanes `owner` carries — `TrackSummary::automation_lanes`.
///
/// Called once per track by the summaries, so it resolves owners with a
/// plain slot scan rather than building a [`LaneContext`] (which clones
/// every chain's parameter list).
pub(in crate::update::control) fn lane_count(app: &Resonance, owner: ChainOwner) -> usize {
    app.automation
        .lanes
        .keys()
        .filter(|target| lane_owner(app, target) == Some(owner))
        .count()
}

/// The owner of the lane on `target`: the strip for a mixer lane, the
/// chain hosting the instance for a plugin lane (`None` when it is gone).
pub(in crate::update::control) fn lane_owner(
    app: &Resonance,
    target: &AutomationTarget,
) -> Option<ChainOwner> {
    match target {
        AutomationTarget::TrackGain(id)
        | AutomationTarget::TrackPan(id)
        | AutomationTarget::TrackMute(id)
        | AutomationTarget::DeviceParam { track: id, .. } => Some(ChainOwner::Track(*id)),
        AutomationTarget::BusGain(id)
        | AutomationTarget::BusPan(id)
        | AutomationTarget::BusMute(id) => Some(ChainOwner::Bus(*id)),
        AutomationTarget::MasterGain => Some(ChainOwner::Master),
        AutomationTarget::PluginParam { instance, .. } => {
            let hosts = |plugins: &[crate::state::PluginSlotState]| {
                plugins.iter().any(|p| p.instance_id == *instance)
            };
            if let Some(t) = app.registry.tracks.iter().find(|t| hosts(&t.plugins)) {
                return Some(ChainOwner::Track(t.id));
            }
            if let Some(b) = app.registry.busses.iter().find(|b| hosts(&b.plugins)) {
                return Some(ChainOwner::Bus(b.id));
            }
            hosts(&app.master.plugins).then_some(ChainOwner::Master)
        }
    }
}

/// The model's curve as its wire form.
pub(in crate::update::control) fn wire_curve(curve: CurveKind) -> AutomationCurve {
    match curve {
        CurveKind::Linear => AutomationCurve::Linear,
        CurveKind::Stepped => AutomationCurve::Stepped,
    }
}

/// The wire's curve as the model's.
pub(in crate::update::control) fn model_curve(curve: AutomationCurve) -> CurveKind {
    match curve {
        AutomationCurve::Linear => CurveKind::Linear,
        AutomationCurve::Stepped => CurveKind::Stepped,
    }
}
