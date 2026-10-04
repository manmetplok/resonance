//! Plugin addressing shared by every chain owner (automation-control-api.md
//! §4.1).
//!
//! `track.set_plugin_param`, `bus.set_plugin_param`,
//! `master.set_plugin_param` and the `automation.*` plugin lanes all name
//! a parameter the same way: an owner, an optional CLAP `plugin_id` +
//! `occurrence`, and a `param` string. They used to resolve it in three
//! hand-written copies; this is the one resolution, so a plugin and a
//! parameter are found — and missed — with the same words everywhere.

use super::reply::{no_bus, no_track};
use super::view_model;
use crate::state::PluginSlotState;
use crate::Resonance;
use resonance_audio::types::PluginInstanceId;
use resonance_control::methods::track::{self, PluginKind, PluginParamView, PluginParamsEntry};
use resonance_control::RpcError;

/// Which chain a plugin sits on: the engine's own owner type (ARCH2-02),
/// re-exported so the control layer keeps naming it from here.
pub(crate) use resonance_audio::types::ChainOwner;

/// The owner's `*.plugin_params` method, for retry advice.
fn params_method(owner: ChainOwner) -> &'static str {
    match owner {
        ChainOwner::Track(_) => "track.plugin_params",
        ChainOwner::Bus(_) => "bus.plugin_params",
        ChainOwner::Master => "master.plugin_params",
    }
}

/// The owner as the "vanished from ..." object.
fn chain_name(owner: ChainOwner) -> String {
    match owner {
        ChainOwner::Track(id) => format!("track {id}"),
        ChainOwner::Bus(id) => format!("bus {id}"),
        ChainOwner::Master => "the master chain".to_owned(),
    }
}

/// One resolved plugin: its wire entry and its engine handle.
#[derive(Debug, Clone)]
pub(crate) struct PluginTarget {
    pub entry: PluginParamsEntry,
    pub instance_id: PluginInstanceId,
}

/// The owner's chain as live slots, or the owner's `not_found`.
pub(crate) fn chain_slots(
    app: &Resonance,
    owner: ChainOwner,
) -> Result<&[PluginSlotState], RpcError> {
    app.chain(owner).ok_or_else(|| match owner {
        ChainOwner::Track(id) => no_track(id),
        ChainOwner::Bus(id) => no_bus(id),
        ChainOwner::Master => unreachable!("the master chain always exists"),
    })
}

/// The owner's chain as wire entries, or the owner's `not_found`.
pub(crate) fn chain_entries(
    app: &Resonance,
    owner: ChainOwner,
) -> Result<Vec<PluginParamsEntry>, RpcError> {
    match owner {
        ChainOwner::Track(id) => app
            .registry
            .tracks
            .iter()
            .find(|t| t.id == id)
            .map(|t| view_model::plugin_entries(app, t))
            .ok_or_else(|| no_track(id)),
        ChainOwner::Bus(id) => app
            .registry
            .busses
            .iter()
            .find(|b| b.id == id)
            .map(view_model::bus_plugin_entries)
            .ok_or_else(|| no_bus(id)),
        ChainOwner::Master => Ok(view_model::master_plugin_entries(app)),
    }
}

/// Resolve `plugin_id` + `occurrence` on `owner`'s chain.
///
/// An omitted `plugin_id` names the track's instrument, or — on a bus or
/// the master, which have none — the first plugin in the chain, which is
/// unambiguous on the common one-effect chain.
pub(crate) fn resolve_plugin_target(
    app: &Resonance,
    owner: ChainOwner,
    plugin_id: Option<&str>,
    occurrence: Option<u32>,
) -> Result<PluginTarget, RpcError> {
    let entries = chain_entries(app, owner)?;
    let occurrence = occurrence.unwrap_or(0);
    let entry = match (plugin_id, owner) {
        (Some(id), _) => entries
            .iter()
            .find(|e| e.plugin_id == id && e.occurrence == occurrence),
        (None, ChainOwner::Track(_)) => entries.iter().find(|e| e.kind == PluginKind::Instrument),
        (None, _) => entries.first(),
    };
    let Some(entry) = entry.cloned() else {
        return Err(missing_plugin(app, owner, plugin_id, occurrence, &entries));
    };
    let slots = chain_slots(app, owner)?;
    let Some(instance_id) =
        super::effect_addressing::instance_at(slots, &entry.plugin_id, entry.occurrence)
    else {
        return Err(RpcError::not_found(format!(
            "plugin {:?} vanished from {} between lookup and set",
            entry.plugin_id,
            chain_name(owner)
        )));
    };
    Ok(PluginTarget { entry, instance_id })
}

/// [`resolve_plugin_target`], then the parameter on it.
///
/// Refuses a plugin whose parameter list is still empty with `busy` —
/// the slot is mirrored at dispatch, the list arrives with the engine
/// echo (todo #1234) — and a parameter [`super::track::find_param`]
/// cannot find with `not_found` listing the names it has.
pub(crate) fn resolve_plugin_param(
    app: &Resonance,
    owner: ChainOwner,
    plugin_id: Option<&str>,
    occurrence: Option<u32>,
    param: &str,
) -> Result<(PluginTarget, PluginParamView), RpcError> {
    let target = resolve_plugin_target(app, owner, plugin_id, occurrence)?;
    if target.entry.params.is_empty() {
        return Err(initializing(owner, &target.entry.plugin_id));
    }
    let wanted = param.trim();
    let Some(param) = super::track::find_param(&target.entry.params, wanted).cloned() else {
        return Err(unknown_param(&target.entry, wanted));
    };
    Ok((target, param))
}

/// "Plugin X is on OWNER but is still initializing" — the `busy` a
/// freshly added plugin answers until its parameter list arrives.
pub(crate) fn initializing(owner: ChainOwner, plugin_id: &str) -> RpcError {
    RpcError::busy(format!(
        "plugin {plugin_id:?} is on {} but is still initializing — its parameter list \
         arrives with the engine echo, usually within a frame. Retry, or read \
         {} until its params array is non-empty. (A plugin that \
         genuinely exposes no parameters reports the same empty list.)",
        owner,
        params_method(owner)
    ))
}

/// "Plugin X has no parameter Y (has: [...])".
pub(crate) fn unknown_param(entry: &PluginParamsEntry, wanted: &str) -> RpcError {
    let known: Vec<&str> = entry.params.iter().map(|p| p.name.as_str()).collect();
    RpcError::not_found(format!(
        "plugin {:?} has no parameter {wanted:?} (has: [{}])",
        entry.plugin_id,
        known.join(", ")
    ))
}

/// The miss for a plugin address, worded per owner exactly as the
/// `*.set_plugin_param` methods always worded it.
fn missing_plugin(
    app: &Resonance,
    owner: ChainOwner,
    plugin_id: Option<&str>,
    occurrence: u32,
    entries: &[track::PluginParamsEntry],
) -> RpcError {
    match (owner, plugin_id) {
        (ChainOwner::Track(id), Some(wanted)) => {
            match app.registry.tracks.iter().find(|t| t.id == id) {
                Some(t) => view_model::unknown_plugin_on_track(app, t, wanted, occurrence),
                None => no_track(id),
            }
        }
        (ChainOwner::Track(id), None) => RpcError::invalid_params(format!(
            "track {id} has no instrument; name a plugin_id (it carries: [{}])",
            entries
                .iter()
                .map(|e| e.plugin_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        (ChainOwner::Bus(id), Some(wanted)) => RpcError::not_found(format!(
            "bus {id} has no plugin {wanted:?} at occurrence {occurrence}; it carries [{}]",
            super::effect_addressing::chain_description(entries)
        )),
        (ChainOwner::Bus(id), None) => RpcError::not_found(format!(
            "bus {id} carries no plugins; add one with bus.add_effect"
        )),
        (ChainOwner::Master, Some(wanted)) => RpcError::not_found(format!(
            "the master chain has no plugin {wanted:?} at occurrence {occurrence}; it \
             carries [{}]",
            super::effect_addressing::chain_description(entries)
        )),
        (ChainOwner::Master, None) => RpcError::not_found(
            "the master chain carries no plugins; add one with master.add_effect",
        ),
    }
}
