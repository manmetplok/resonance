//! Track-plugin handlers: add/remove instances, parameter writes,
//! GUI open/close, individual + bulk state save/load. Any handler that
//! touches a plugin instance must `try_lock` — if the audio callback
//! holds the lock, the command gets re-enqueued via `cmd_tx_retry` so
//! the audio thread is never blocked.

use std::path::Path;
use std::sync::Arc;

use crate::clap_host::{ClapBundle, SyncClapInstance};
use crate::types::*;

use super::thread::{HandlerCtx, HandlerState};

/// True for commands that can change a track's or bus's chain latency
/// (plugin add/remove, routing, track/bus topology). The engine loop
/// republishes the plugin-delay-compensation table after these run.
pub(crate) fn affects_latency(cmd: &AudioCommand) -> bool {
    matches!(
        cmd,
        AudioCommand::AddPlugin { .. }
            | AudioCommand::RemovePlugin { .. }
            | AudioCommand::AddPluginToBus { .. }
            | AudioCommand::RemovePluginFromBus { .. }
            | AudioCommand::ScanPlugins
            | AudioCommand::SetTrackOutput { .. }
            | AudioCommand::AddTrack { .. }
            | AudioCommand::AddInstrumentTrack { .. }
            | AudioCommand::AddVocalTrack { .. }
            | AudioCommand::CreateSubTrack { .. }
            | AudioCommand::RemoveTrack { .. }
            | AudioCommand::AddBus { .. }
            | AudioCommand::RemoveBus { .. }
            | AudioCommand::ClearAll
    )
}

/// Recompute per-track compensation delays from the current topology
/// and publish a fresh table for the audio callback. Skips the publish
/// (and thus the delay-line reset) when no delay actually changed.
/// Runs on the engine thread; delay lines are allocated here, never on
/// the audio callback.
pub(crate) fn refresh_latency_comp(ctx: &HandlerCtx) {
    let chains = {
        let tracks_guard = ctx.tracks.read();
        let busses_guard = ctx.busses.read();
        let plugins_guard = ctx.plugins.read();
        crate::latency::chain_latencies(&tracks_guard, &busses_guard, |id| {
            plugins_guard
                .get(&id)
                .map(|m| super::try_lock_with_backoff(m).0.latency_samples() as u64)
                .unwrap_or(0)
        })
    };
    let (max, delays) = crate::latency::compensation_delays(&chains);
    if ctx.latency_comp.load().delays_match(&delays) {
        return;
    }
    ctx.latency_comp
        .store(Arc::new(crate::latency::LatencyComp::new(max, &delays)));
}

pub(crate) fn handle_add_plugin(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
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

            // Query params + has_gui + output port layout before moving
            // instance into shared map.
            let params = instance.query_params();
            let has_gui = instance.has_gui();
            let output_port_count = instance.output_port_count();
            let output_port_names = instance.output_port_names();

            ctx.plugins.write().insert(
                instance_id,
                parking_lot::Mutex::new(SyncClapInstance(instance)),
            );

            // `push_plugin` publishes the new chain via `ArcSwap::store`,
            // so we only need a read guard — the audio thread is not
            // blocked while the chain edit happens.
            if let Some(track) = ctx.tracks.read().get(&track_id) {
                track.push_plugin(instance_id);
            }

            let _ = ctx.event_tx.send(AudioEvent::PluginAdded {
                track_id,
                instance_id,
                plugin_name,
                clap_plugin_id: actual_plugin_id,
                clap_file_path,
                params,
                has_gui,
                output_port_count,
                output_port_names,
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

pub(crate) fn handle_remove_plugin(
    ctx: &HandlerCtx,
    track_id: TrackId,
    instance_id: PluginInstanceId,
) {
    // `retain_plugins` publishes a new chain via `ArcSwap::store`, so
    // we only need a read guard on the tracks map — the audio thread
    // is never blocked on the chain edit.
    if let Some(track) = ctx.tracks.read().get(&track_id) {
        track.retain_plugins(|&id| id != instance_id);
    }
    // Remove from map then drop outside the write lock so the audio
    // callback isn't blocked during plugin deactivation.
    let removed = ctx.plugins.write().shift_remove(&instance_id);
    drop(removed);
    let _ = ctx.event_tx.send(AudioEvent::PluginRemoved {
        track_id,
        instance_id,
    });
}

pub(crate) fn handle_set_plugin_param(
    ctx: &HandlerCtx,
    instance_id: PluginInstanceId,
    param_id: u32,
    value: f64,
) {
    if let Some(mutex) = ctx.plugins.read().get(&instance_id) {
        if let Some(mut inst) = mutex.try_lock() {
            inst.0.set_param(param_id, value);
        } else {
            // Audio thread is mid-process(): re-enqueue so the param
            // change lands on the next iteration rather than blocking
            // here. Blocking causes the audio thread's own try_lock to
            // start failing too, which silences the plugin for a block.
            let _ = ctx.cmd_tx_retry.send(AudioCommand::SetPluginParam {
                instance_id,
                param_id,
                value,
            });
        }
    }
}

pub(crate) fn handle_open_plugin_editor(ctx: &HandlerCtx, instance_id: PluginInstanceId) {
    if let Some(mutex) = ctx.plugins.read().get(&instance_id) {
        // open_gui is a main-thread operation; the audio thread holds
        // a different lock. Block briefly if the audio thread is
        // mid-process and retry.
        if let Some(mut inst) = mutex.try_lock() {
            if !inst.0.open_gui() {
                let _ = ctx.event_tx.send(AudioEvent::Error(
                    "Failed to open plugin editor".to_string(),
                ));
            }
        } else {
            let _ = ctx
                .cmd_tx_retry
                .send(AudioCommand::OpenPluginEditor { instance_id });
        }
    }
}

pub(crate) fn handle_close_plugin_editor(ctx: &HandlerCtx, instance_id: PluginInstanceId) {
    if let Some(mutex) = ctx.plugins.read().get(&instance_id) {
        if let Some(mut inst) = mutex.try_lock() {
            inst.0.close_gui();
        } else {
            let _ = ctx
                .cmd_tx_retry
                .send(AudioCommand::ClosePluginEditor { instance_id });
        }
    }
}

pub(crate) fn handle_save_plugin_state(ctx: &HandlerCtx, instance_id: PluginInstanceId) {
    if let Some(mutex) = ctx.plugins.read().get(&instance_id) {
        if let Some(inst) = mutex.try_lock() {
            let data = inst.0.save_state();
            if let Some(data) = data {
                let _ = ctx
                    .event_tx
                    .send(AudioEvent::PluginStateSaved { instance_id, data });
            }
        } else {
            // Audio thread holds the lock — retry next tick
            let _ = ctx
                .cmd_tx_retry
                .send(AudioCommand::SavePluginState { instance_id });
        }
    }
}

pub(crate) fn handle_load_plugin_state(
    ctx: &HandlerCtx,
    instance_id: PluginInstanceId,
    data: Vec<u8>,
) {
    if let Some(mutex) = ctx.plugins.read().get(&instance_id) {
        if let Some(mut inst) = mutex.try_lock() {
            inst.0.reload_with_state(&data);
        } else {
            // Audio thread holds the lock — retry next tick
            let _ = ctx
                .cmd_tx_retry
                .send(AudioCommand::LoadPluginState { instance_id, data });
        }
    }
}

pub(crate) fn handle_save_all_plugin_states(ctx: &HandlerCtx) {
    let mut states = Vec::new();
    let plugins_guard = ctx.plugins.read();
    let mut retry = false;
    for (&instance_id, mutex) in plugins_guard.iter() {
        if let Some(inst) = mutex.try_lock() {
            if let Some(data) = inst.0.save_state() {
                states.push((instance_id, data));
            }
        } else {
            retry = true;
            break;
        }
    }
    drop(plugins_guard);
    if retry {
        let _ = ctx.cmd_tx_retry.send(AudioCommand::SaveAllPluginStates);
    } else {
        let _ = ctx
            .event_tx
            .send(AudioEvent::AllPluginStatesSaved { states });
    }
}

/// Returns the index of the bundle that owns `clap_plugin_id`, loading
/// the file from disk if needed. Sends an error event and returns `None`
/// on load failure.
pub(crate) fn ensure_bundle(
    bundles: &mut Vec<ClapBundle>,
    path: &Path,
    clap_plugin_id: &str,
    ctx: &HandlerCtx,
) -> Option<usize> {
    if let Some(idx) = bundles
        .iter()
        .position(|b| b.descriptors().iter().any(|d| d.id == clap_plugin_id))
    {
        return Some(idx);
    }
    match ClapBundle::load(path) {
        Ok(bundle) => {
            bundles.push(bundle);
            Some(bundles.len() - 1)
        }
        Err(e) => {
            let _ = ctx
                .event_tx
                .send(AudioEvent::Error(format!("Failed to load plugin: {}", e)));
            None
        }
    }
}

/// Returns the canonical plugin id to instantiate from `bundle`. If the
/// caller passed an empty id, pick the first descriptor; otherwise hand
/// back the id as-is.
pub(crate) fn resolve_plugin_id(
    bundle: &ClapBundle,
    clap_plugin_id: String,
    ctx: &HandlerCtx,
) -> Option<String> {
    if !clap_plugin_id.is_empty() {
        return Some(clap_plugin_id);
    }
    match bundle.descriptors().first() {
        Some(d) => Some(d.id.clone()),
        None => {
            let _ = ctx
                .event_tx
                .send(AudioEvent::Error("No plugins found in file".to_string()));
            None
        }
    }
}
