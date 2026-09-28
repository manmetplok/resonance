//! Wire target → model target (automation-control-api.md §4.1).
//!
//! One resolution for every `automation.*` method, built on the shared
//! plugin addressing ([`resolve_plugin_param`]) so a plugin lane's
//! target is found — and missed — with the words `*.set_plugin_param`
//! uses.

use super::value::ValueDomain;
use crate::update::control::plugin_target::{resolve_plugin_param, ChainOwner, PluginTarget};
use crate::update::control::reply::{no_bus, no_track};
use crate::update::control::track::frozen_reject;
use crate::Resonance;
use resonance_common::AutomationTarget;
use resonance_control::methods::automation::{AutomationTargetSpec, LaneControl};
use resonance_control::RpcError;

/// A wire target resolved against the open project.
#[derive(Debug, Clone)]
pub(in crate::update::control) struct ResolvedTarget {
    /// The model's lane key.
    pub target: AutomationTarget,
    /// What the lane's values mean.
    pub domain: ValueDomain,
    /// The chain / strip the lane belongs to.
    pub owner: ChainOwner,
    /// The plugin, for a plugin lane.
    pub plugin: Option<PluginTarget>,
}

/// The owner a spec names, if any: `Ok(None)` when it names none (a
/// listing filter may), an error when it names more than one or an id
/// that does not exist.
pub(in crate::update::control) fn spec_owner(
    app: &Resonance,
    spec: &AutomationTargetSpec,
) -> Result<Option<ChainOwner>, RpcError> {
    let named = usize::from(spec.track_id.is_some())
        + usize::from(spec.bus_id.is_some())
        + usize::from(spec.master);
    if named > 1 {
        return Err(RpcError::invalid_params(
            "name exactly one owner: track_id, bus_id or master: true",
        ));
    }
    if let Some(id) = spec.track_id {
        if !app.registry.tracks.iter().any(|t| t.id == id.0) {
            return Err(no_track(id.0));
        }
        return Ok(Some(ChainOwner::Track(id.0)));
    }
    if let Some(id) = spec.bus_id {
        if !app.registry.busses.iter().any(|b| b.id == id.0) {
            return Err(no_bus(id.0));
        }
        return Ok(Some(ChainOwner::Bus(id.0)));
    }
    Ok(spec.master.then_some(ChainOwner::Master))
}

/// Resolve a lane address: exactly one owner, exactly one of `control` /
/// `param`. Read-side and write-side alike; writes go through
/// [`resolve_write_target`], which adds the frozen-track rule.
pub(in crate::update::control) fn resolve_target(
    app: &Resonance,
    spec: &AutomationTargetSpec,
) -> Result<ResolvedTarget, RpcError> {
    let Some(owner) = spec_owner(app, spec)? else {
        return Err(RpcError::invalid_params(
            "name the lane's owner: track_id, bus_id or master: true",
        ));
    };
    match (spec.control, spec.param.as_deref()) {
        (Some(_), Some(_)) => Err(RpcError::invalid_params(
            "give either control (volume / pan / mute) or param (a plugin parameter), not \
             both — a plugin parameter called \"Volume\" is a param",
        )),
        (None, None) => Err(RpcError::invalid_params(
            "name the lane: control (\"volume\", \"pan\", \"mute\") or param (a plugin \
             parameter, with plugin_id / occurrence when it is not the track's instrument)",
        )),
        (Some(control), None) => {
            if spec.plugin_id.is_some() || spec.occurrence.is_some() {
                return Err(RpcError::invalid_params(
                    "plugin_id / occurrence address a plugin lane (param); a control lane \
                     takes neither",
                ));
            }
            let target = mixer_target(owner, control)?;
            Ok(ResolvedTarget {
                domain: ValueDomain::for_mixer(&target),
                target,
                owner,
                plugin: None,
            })
        }
        (None, Some(param)) => {
            let (plugin, param) = resolve_plugin_param(
                app,
                owner,
                spec.plugin_id.as_deref(),
                spec.occurrence,
                param,
            )?;
            Ok(ResolvedTarget {
                target: AutomationTarget::PluginParam {
                    instance: plugin.instance_id,
                    param_id: param.id,
                },
                domain: ValueDomain::Plugin(param),
                owner,
                plugin: Some(plugin),
            })
        }
    }
}

/// [`resolve_target`] for a method that WRITES the lane: a plugin lane on
/// a frozen track is refused with the `frozen_reject` error
/// `track.set_plugin_param` gives (automation-control-api.md D2) — the
/// frozen cache plays, so the edit would be inaudible, and the gate in
/// `update()` would swallow it anyway. Mixer lanes stay writable: the
/// mixer runs live while frozen.
pub(in crate::update::control) fn resolve_write_target(
    app: &Resonance,
    spec: &AutomationTargetSpec,
) -> Result<ResolvedTarget, RpcError> {
    let resolved = resolve_target(app, spec)?;
    if let (Some(_), ChainOwner::Track(track_id)) = (&resolved.plugin, resolved.owner) {
        if let Some(e) = frozen_reject(app, track_id) {
            return Err(e);
        }
    }
    Ok(resolved)
}

/// The mixer lane `control` names on `owner`.
fn mixer_target(owner: ChainOwner, control: LaneControl) -> Result<AutomationTarget, RpcError> {
    Ok(match (owner, control) {
        (_, LaneControl::Device) => {
            return Err(RpcError::invalid_params(
                "device lanes (external-instrument parameters) are read-only here; \
                 automation.lanes lists them",
            ))
        }
        (ChainOwner::Track(id), LaneControl::Volume) => AutomationTarget::TrackGain(id),
        (ChainOwner::Track(id), LaneControl::Pan) => AutomationTarget::TrackPan(id),
        (ChainOwner::Track(id), LaneControl::Mute) => AutomationTarget::TrackMute(id),
        (ChainOwner::Bus(id), LaneControl::Volume) => AutomationTarget::BusGain(id),
        (ChainOwner::Bus(id), LaneControl::Pan) => AutomationTarget::BusPan(id),
        (ChainOwner::Bus(id), LaneControl::Mute) => AutomationTarget::BusMute(id),
        (ChainOwner::Master, LaneControl::Volume) => AutomationTarget::MasterGain,
        (ChainOwner::Master, LaneControl::Pan | LaneControl::Mute) => {
            return Err(RpcError::invalid_params(
                "the master has only a volume lane (control: \"volume\")",
            ))
        }
    })
}
