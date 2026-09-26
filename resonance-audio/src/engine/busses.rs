//! Bus handlers: create/destroy, per-bus volume/pan/mute/name,
//! track→bus routing, and bus-owned plugin CRUD. Bus plugin
//! add/remove reuses `ensure_bundle` / `resolve_plugin_id` from
//! the `plugins` module.

use std::path::Path;

use indexmap::IndexMap;

use crate::types::*;

use super::plugins::{
    ensure_bundle, reject_if_plugin_id_in_use, report_plugin_load_failure, resolve_plugin_id,
};
use super::thread::{HandlerCtx, HandlerState};
use super::MAX_BUSSES;

/// Refuse an add whose id is already live in `busses`, rather than
/// silently replacing the bus it names — the bus twin of
/// `plugins::reject_if_plugin_id_in_use` (ARCH-04 D-3). Takes the guard
/// already held by [`handle_add_bus`] rather than re-locking `ctx.busses`,
/// so the whole add stays one atomic critical section.
fn reject_if_bus_id_in_use(ctx: &HandlerCtx, busses: &IndexMap<BusId, Bus>, id: BusId) -> bool {
    if busses.contains_key(&id) {
        let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::internal(format!(
            "bus id {id} is already in use; refusing the add rather than replacing the live bus"
        ))));
        true
    } else {
        false
    }
}

pub(crate) fn handle_add_bus(ctx: &HandlerCtx, id: BusId, name: Option<String>) {
    let mut busses_guard = ctx.busses.write();
    if busses_guard.len() >= MAX_BUSSES {
        let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::busy(format!(
            "Cannot add bus: maximum of {MAX_BUSSES} busses reached"
        ))));
        return;
    }
    if reject_if_bus_id_in_use(ctx, &busses_guard, id) {
        return;
    }
    let name = name.unwrap_or_else(|| format!("Bus {id}"));
    busses_guard.insert(id, Bus::new(id, name.clone()));
    drop(busses_guard);
    let _ = ctx.event_tx.send(AudioEvent::BusAdded { bus_id: id, name });
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
        super::plugins::apply_bypass_request(ctx.shared, bus.fx_bypass(), bypassed);
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
    id: PluginInstanceId,
) {
    if reject_if_plugin_id_in_use(ctx, id, &clap_plugin_id) {
        return;
    }
    let path = Path::new(&clap_file_path);
    let bundle_idx = match ensure_bundle(&mut state.bundles, path, &clap_plugin_id) {
        Ok(idx) => idx,
        Err(reason) => {
            report_plugin_load_failure(ctx, Some(id), &clap_plugin_id, &clap_file_path, reason);
            return;
        }
    };
    let actual_plugin_id =
        match resolve_plugin_id(&state.bundles[bundle_idx], clap_plugin_id.clone()) {
            Ok(resolved) => resolved,
            Err(reason) => {
                report_plugin_load_failure(ctx, Some(id), &clap_plugin_id, &clap_file_path, reason);
                return;
            }
        };
    let plugin_name = state.bundles[bundle_idx]
        .descriptors()
        .iter()
        .find(|d| d.id == actual_plugin_id)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| actual_plugin_id.clone());
    match state.bundles[bundle_idx].create_instance(&actual_plugin_id, ctx.sample_rate) {
        Ok(instance) => {
            let instance_id = id;
            let params = instance.query_params();
            let has_gui = instance.has_gui();
            let has_sidechain_input = instance.has_sidechain_input();
            ctx.plugins.write().insert(
                instance_id,
                crate::clap_host::PluginSlot::new(instance),
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
                has_sidechain_input,
            });
        }
        Err(e) => report_plugin_load_failure(
            ctx,
            Some(id),
            &actual_plugin_id,
            &clap_file_path,
            format!("Failed to create plugin instance: {}", e),
        ),
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

/// Reorder a bus's insert chain (ba doc #273, todo #1237).
///
/// A bus chain is a plain `Vec<PluginInstanceId>` behind the busses
/// write lock, NOT the `ArcSwap` a track chain uses, so this mirrors
/// `handle_remove_plugin_from_bus`'s pattern (one short write guard, no
/// plugin instance touched — only the order they are visited in) rather
/// than `handle_move_plugin`'s publish-a-new-Arc pattern.
pub(crate) fn handle_move_plugin_in_bus(
    ctx: &HandlerCtx,
    bus_id: BusId,
    instance_id: PluginInstanceId,
    to_index: usize,
) {
    let moved = ctx
        .busses
        .write()
        .get_mut(&bus_id)
        .and_then(|bus| bus.move_plugin(instance_id, to_index));
    match moved {
        // Report the *clamped* index so the app mirrors what the engine
        // actually did rather than what was requested.
        Some(to_index) => {
            let _ = ctx.event_tx.send(AudioEvent::BusPluginMoved {
                bus_id,
                instance_id,
                to_index,
            });
        }
        None => {
            let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::not_found(format!(
                "Cannot reorder plugin {} on bus {}: no such bus, or that \
                 plugin is not on its chain",
                instance_id, bus_id
            ))));
        }
    }
}

/// Lower / upper bound on aux-send level in dB. Mirrors the spirit of
/// `handle_set_clip_gain`'s clamp: keep stored values finite and sane so
/// a stray `NaN`/`inf` from the GUI can never poison engine state.
const AUX_SEND_MIN_DB: f32 = -120.0;
const AUX_SEND_MAX_DB: f32 = 24.0;

/// Republish the control-thread aux-send table into the lock-free
/// snapshot the audio callback and bounce renderer read each block (see
/// [`SharedState::aux_sends`](crate::engine::SharedState)). Called after
/// every mutation of `state.aux_sends` so the render path never sees a
/// stale or partially-updated table.
pub(crate) fn publish_aux_sends(ctx: &HandlerCtx, state: &HandlerState) {
    let snapshot: Vec<AuxSend> = state.aux_sends.values().copied().collect();
    super::retire::publish(
        &ctx.shared.aux_sends,
        std::sync::Arc::new(snapshot),
        &ctx.shared.retired,
    );
}

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

/// Validate a prospective aux-send route: the destination and source must
/// exist, and registering it must not close a feedback loop. Shared by
/// [`handle_add_aux_send`] and [`handle_set_aux_send`] (ARCH-04 D-2) — the
/// only difference between the two is `updating`, which excludes a send's
/// own current edge from the cycle check: `None` for a brand-new send,
/// `Some(id)` for an edit of an existing one. Reports
/// `AudioEvent::AuxSendRejected` and returns `false` on any failure;
/// `true` means the route is safe to register.
fn validate_aux_send_route(
    ctx: &HandlerCtx,
    state: &HandlerState,
    updating: Option<SendId>,
    source: SendSource,
    dest: BusId,
) -> bool {
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
        return false;
    }
    // Source must exist (a track or a bus, depending on the variant).
    match source {
        SendSource::Track(tid) => {
            if !ctx.tracks.read().contains_key(&tid) {
                reject(format!("Aux send source track {tid} does not exist"));
                return false;
            }
        }
        SendSource::Bus(bid) => {
            if !ctx.busses.read().contains_key(&bid) {
                reject(format!("Aux send source bus {bid} does not exist"));
                return false;
            }
        }
    }

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
        return false;
    }
    true
}

/// Clamp a send level to the sane range, treating a non-finite value (a
/// stray `NaN`/`inf` from the GUI) as unity rather than storing it.
fn clamp_send_level(level_db: f32) -> f32 {
    if level_db.is_finite() {
        level_db.clamp(AUX_SEND_MIN_DB, AUX_SEND_MAX_DB)
    } else {
        0.0
    }
}

/// Refuse an `AddAuxSend` whose id is already live, rather than silently
/// turning what the caller meant as a create into an edit of the send that
/// id already names — the send twin of `plugins::reject_if_plugin_id_in_use`
/// (ARCH-04 D-2).
fn reject_if_send_id_in_use(ctx: &HandlerCtx, state: &HandlerState, id: SendId) -> bool {
    if state.aux_sends.contains_key(&id) {
        let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::internal(format!(
            "aux send id {id} is already in use; refusing the add rather than replacing the \
             live send"
        ))));
        true
    } else {
        false
    }
}

/// Create a new aux send under an app-allocated `id` (ARCH-04 D-2). Refuses
/// a colliding id instead of silently editing the send it already names —
/// see [`reject_if_send_id_in_use`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_add_aux_send(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    id: SendId,
    source: SendSource,
    dest: BusId,
    level_db: f32,
    pre_fader: bool,
    enabled: bool,
) {
    if reject_if_send_id_in_use(ctx, state, id) {
        return;
    }
    if !validate_aux_send_route(ctx, state, None, source, dest) {
        return;
    }
    let level_db = clamp_send_level(level_db);
    state.aux_sends.insert(
        id,
        AuxSend {
            id,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        },
    );
    // Make the new send visible to the audio + bounce render paths before
    // confirming the change to the app.
    publish_aux_sends(ctx, state);
    let _ = ctx.event_tx.send(AudioEvent::AuxSendChanged {
        send_id: id,
        source,
        dest,
        level_db,
        pre_fader,
        enabled,
    });
}

/// Edit an existing aux send in place — re-route / level / pre-post /
/// enable, all covered by resending the send's full state under its own
/// `id` (ARCH-04 D-2). A quiet no-op if `id` does not name a live send (an
/// edit racing its own removal): unlike a duplicate id on
/// [`handle_add_aux_send`], there is no caller invariant to complain about
/// here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_set_aux_send(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    id: SendId,
    source: SendSource,
    dest: BusId,
    level_db: f32,
    pre_fader: bool,
    enabled: bool,
) {
    if !state.aux_sends.contains_key(&id) {
        return;
    }
    if !validate_aux_send_route(ctx, state, Some(id), source, dest) {
        return;
    }
    let level_db = clamp_send_level(level_db);
    state.aux_sends.insert(
        id,
        AuxSend {
            id,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        },
    );
    // Make the updated send visible to the audio + bounce render paths
    // before confirming the change to the app.
    publish_aux_sends(ctx, state);
    let _ = ctx.event_tx.send(AudioEvent::AuxSendChanged {
        send_id: id,
        source,
        dest,
        level_db,
        pre_fader,
        enabled,
    });
}

pub(crate) fn handle_remove_aux_send(ctx: &HandlerCtx, state: &mut HandlerState, send_id: SendId) {
    if state.aux_sends.shift_remove(&send_id).is_some() {
        publish_aux_sends(ctx, state);
        let _ = ctx.event_tx.send(AudioEvent::AuxSendRemoved { send_id });
    }
}
