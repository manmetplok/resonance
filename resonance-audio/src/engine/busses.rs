//! Bus handlers: create/destroy, per-bus volume/pan/mute/name,
//! track→bus routing, and bus-owned plugin CRUD. Bus plugin
//! add/remove reuses `ensure_bundle` / `resolve_plugin_id` from
//! the `plugins` module.

use std::path::Path;

use crate::clap_host::SyncClapInstance;
use crate::types::*;

use super::plugins::{ensure_bundle, resolve_plugin_id};
use super::thread::{HandlerCtx, HandlerState};
use super::MAX_BUSSES;

pub(crate) fn handle_add_bus(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    id_hint: Option<BusId>,
    name: Option<String>,
) {
    let mut busses_guard = ctx.busses.write();
    if busses_guard.len() >= MAX_BUSSES {
        let _ = ctx.event_tx.send(AudioEvent::Error(format!(
            "Cannot add bus: maximum of {MAX_BUSSES} busses reached"
        )));
        return;
    }
    let bus_id = id_hint.unwrap_or_else(|| {
        let i = state.next_bus_id;
        state.next_bus_id += 1;
        i
    });
    if id_hint.is_some() {
        if busses_guard.contains_key(&bus_id) {
            return;
        }
        state.next_bus_id = state.next_bus_id.max(bus_id + 1);
    }
    let name = name.unwrap_or_else(|| format!("Bus {bus_id}"));
    busses_guard.insert(bus_id, Bus::new(bus_id, name.clone()));
    drop(busses_guard);
    let _ = ctx.event_tx.send(AudioEvent::BusAdded { bus_id, name });
}

pub(crate) fn handle_remove_bus(ctx: &HandlerCtx, bus_id: BusId) {
    // First: unassign any track that was routed here so no dangling
    // references survive the removal.
    {
        let tracks_guard = ctx.tracks.read();
        for track in tracks_guard.values() {
            if track.output() == TrackOutput::Bus(bus_id) {
                track.set_output(TrackOutput::Master);
            }
        }
    }
    // Collect the bus's plugin ids before removing it so we can tear
    // them down outside the busses lock.
    let removed_plugins: Vec<PluginInstanceId> = {
        let mut busses_guard = ctx.busses.write();
        if let Some(bus) = busses_guard.shift_remove(&bus_id) {
            bus.plugin_ids
        } else {
            Vec::new()
        }
    };
    // Drop plugin instances off the audio path.
    {
        let mut plugins_guard = ctx.plugins.write();
        for pid in &removed_plugins {
            if let Some(inst) = plugins_guard.shift_remove(pid) {
                drop(inst);
            }
        }
    }
    let _ = ctx.event_tx.send(AudioEvent::BusRemoved { bus_id });
}

pub(crate) fn handle_set_bus_volume(ctx: &HandlerCtx, bus_id: BusId, volume: f32) {
    if let Some(bus) = ctx.busses.read().get(&bus_id) {
        bus.set_volume(volume);
    }
}

pub(crate) fn handle_set_bus_pan(ctx: &HandlerCtx, bus_id: BusId, pan: f32) {
    if let Some(bus) = ctx.busses.read().get(&bus_id) {
        bus.set_pan(pan);
    }
}

pub(crate) fn handle_set_bus_mute(ctx: &HandlerCtx, bus_id: BusId, muted: bool) {
    if let Some(bus) = ctx.busses.read().get(&bus_id) {
        bus.set_muted(muted);
    }
}

pub(crate) fn handle_set_bus_fx_bypass(ctx: &HandlerCtx, bus_id: BusId, bypassed: bool) {
    if let Some(bus) = ctx.busses.read().get(&bus_id) {
        bus.set_fx_bypassed(bypassed);
    }
    let _ = ctx
        .event_tx
        .send(AudioEvent::BusFxBypassChanged { bus_id, bypassed });
}

pub(crate) fn handle_set_bus_name(ctx: &HandlerCtx, bus_id: BusId, name: String) {
    if let Some(bus) = ctx.busses.write().get_mut(&bus_id) {
        bus.name = name;
    }
}

pub(crate) fn handle_set_track_output(ctx: &HandlerCtx, track_id: TrackId, output: TrackOutput) {
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        track.set_output(output);
    }
}

pub(crate) fn handle_add_plugin_to_bus(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    bus_id: BusId,
    clap_file_path: String,
    clap_plugin_id: String,
    id_hint: Option<PluginInstanceId>,
) {
    let path = Path::new(&clap_file_path);
    let bundle_idx = match ensure_bundle(&mut state.bundles, path, &clap_plugin_id, ctx) {
        Some(idx) => idx,
        None => return,
    };
    let actual_plugin_id = match resolve_plugin_id(&state.bundles[bundle_idx], clap_plugin_id, ctx)
    {
        Some(id) => id,
        None => return,
    };
    let plugin_name = state.bundles[bundle_idx]
        .descriptors()
        .iter()
        .find(|d| d.id == actual_plugin_id)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| actual_plugin_id.clone());
    match state.bundles[bundle_idx].create_instance(&actual_plugin_id, ctx.sample_rate) {
        Ok(instance) => {
            let instance_id = id_hint.unwrap_or_else(|| {
                let i = state.next_plugin_id;
                state.next_plugin_id += 1;
                i
            });
            if id_hint.is_some() {
                state.next_plugin_id = state.next_plugin_id.max(instance_id + 1);
            }
            let params = instance.query_params();
            let has_gui = instance.has_gui();
            ctx.plugins.write().insert(
                instance_id,
                parking_lot::Mutex::new(SyncClapInstance(instance)),
            );
            if let Some(bus) = ctx.busses.write().get_mut(&bus_id) {
                bus.plugin_ids.push(instance_id);
            }
            let _ = ctx.event_tx.send(AudioEvent::BusPluginAdded {
                bus_id,
                instance_id,
                plugin_name,
                clap_plugin_id: actual_plugin_id,
                clap_file_path,
                params,
                has_gui,
            });
        }
        Err(e) => {
            let _ = ctx.event_tx.send(AudioEvent::Error(format!(
                "Failed to create plugin instance: {}",
                e
            )));
        }
    }
}

pub(crate) fn handle_remove_plugin_from_bus(
    ctx: &HandlerCtx,
    bus_id: BusId,
    instance_id: PluginInstanceId,
) {
    if let Some(bus) = ctx.busses.write().get_mut(&bus_id) {
        bus.plugin_ids.retain(|&id| id != instance_id);
    }
    let removed = ctx.plugins.write().shift_remove(&instance_id);
    drop(removed);
    let _ = ctx.event_tx.send(AudioEvent::BusPluginRemoved {
        bus_id,
        instance_id,
    });
}

/// Lower / upper bound on aux-send level in dB. Mirrors the spirit of
/// `handle_set_clip_gain`'s clamp: keep stored values finite and sane so
/// a stray `NaN`/`inf` from the GUI can never poison engine state.
const AUX_SEND_MIN_DB: f32 = -120.0;
const AUX_SEND_MAX_DB: f32 = 24.0;

pub(crate) fn handle_set_bus_role(ctx: &HandlerCtx, bus_id: BusId, is_return: bool) {
    // Silently no-op on an unknown bus, matching the other bus setters.
    if let Some(bus) = ctx.busses.read().get(&bus_id) {
        bus.set_is_return(is_return);
    } else {
        return;
    }
    let _ = ctx
        .event_tx
        .send(AudioEvent::BusRoleChanged { bus_id, is_return });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_set_aux_send(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    id_hint: Option<SendId>,
    source: SendSource,
    dest: BusId,
    level_db: f32,
    pre_fader: bool,
    enabled: bool,
) {
    // Reject up front with a plain-language reason; never store an
    // invalid send. The app surfaces `reason` to the user.
    let reject = |reason: String| {
        let _ = ctx.event_tx.send(AudioEvent::AuxSendRejected {
            source,
            dest,
            reason,
        });
    };

    // Destination must be a real bus.
    if !ctx.busses.read().contains_key(&dest) {
        reject(format!("Aux send destination bus {dest} does not exist"));
        return;
    }
    // Source must exist (a track or a bus, depending on the variant).
    match source {
        SendSource::Track(tid) => {
            if !ctx.tracks.read().contains_key(&tid) {
                reject(format!("Aux send source track {tid} does not exist"));
                return;
            }
        }
        SendSource::Bus(bid) => {
            if !ctx.busses.read().contains_key(&bid) {
                reject(format!("Aux send source bus {bid} does not exist"));
                return;
            }
        }
    }

    // An upsert on an existing id must not count its own current edge
    // when checking for cycles.
    let updating = id_hint.filter(|id| state.aux_sends.contains_key(id));
    if aux_send_would_cycle(state.aux_sends.values(), source, dest, updating) {
        reject(match source {
            SendSource::Bus(b) if b == dest => {
                format!("A bus cannot send to itself (bus {dest})")
            }
            SendSource::Bus(b) => format!(
                "Aux send from bus {b} to bus {dest} would create a feedback loop"
            ),
            // Unreachable: track sources never cycle.
            SendSource::Track(t) => format!("Aux send from track {t} is invalid"),
        });
        return;
    }

    let level_db = if level_db.is_finite() {
        level_db.clamp(AUX_SEND_MIN_DB, AUX_SEND_MAX_DB)
    } else {
        0.0
    };

    // Resolve the id: honour `id_hint` (update in place, or a project-
    // load hint), else allocate a fresh monotonic id.
    let send_id = match id_hint {
        Some(id) => {
            // Bump the allocator past any hinted id so a later fresh send
            // can't collide with it.
            state.next_send_id = state.next_send_id.max(id + 1);
            id
        }
        None => {
            let id = state.next_send_id;
            state.next_send_id += 1;
            id
        }
    };

    state.aux_sends.insert(
        send_id,
        AuxSend {
            id: send_id,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        },
    );

    let _ = ctx.event_tx.send(AudioEvent::AuxSendChanged {
        send_id,
        source,
        dest,
        level_db,
        pre_fader,
        enabled,
    });
}

pub(crate) fn handle_remove_aux_send(ctx: &HandlerCtx, state: &mut HandlerState, send_id: SendId) {
    if state.aux_sends.shift_remove(&send_id).is_some() {
        let _ = ctx.event_tx.send(AudioEvent::AuxSendRemoved { send_id });
    }
}
