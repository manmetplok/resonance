//! Master-bus plugin handlers. The master bus owns an insert chain that
//! runs after every track and bus has been summed into the output, just
//! before the master volume + clip + peak pass.
//!
//! Mechanically these handlers are a trimmed clone of the bus plugin
//! handlers in `busses.rs` — same bundle lookup, same instance creation,
//! same retry-on-lock pattern via `cmd_tx_retry`. The only difference is
//! that the plugin list lives on `MasterBus` instead of a keyed `Bus`.

use std::path::Path;
use std::sync::atomic::Ordering;

use crate::clap_host::SyncClapInstance;
use crate::types::*;

use super::plugins::{allocate_plugin_instance_id, ensure_bundle, resolve_plugin_id};
use super::thread::{HandlerCtx, HandlerState};

pub(crate) fn handle_add_plugin_to_master(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
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
            let instance_id = allocate_plugin_instance_id(&mut state.next_plugin_id, id_hint);
            let params = instance.query_params();
            let has_gui = instance.has_gui();
            ctx.plugins.write().insert(
                instance_id,
                parking_lot::Mutex::new(SyncClapInstance(instance)),
            );
            ctx.master.write().plugin_ids.push(instance_id);
            let _ = ctx.event_tx.send(AudioEvent::MasterPluginAdded {
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

pub(crate) fn handle_remove_plugin_from_master(ctx: &HandlerCtx, instance_id: PluginInstanceId) {
    ctx.master
        .write()
        .plugin_ids
        .retain(|&id| id != instance_id);
    let removed = ctx.plugins.write().shift_remove(&instance_id);
    drop(removed);
    let _ = ctx
        .event_tx
        .send(AudioEvent::MasterPluginRemoved { instance_id });
}

pub(crate) fn handle_move_plugin_in_master(
    ctx: &HandlerCtx,
    instance_id: PluginInstanceId,
    to_index: usize,
) {
    let moved = ctx.master.write().move_plugin(instance_id, to_index);
    match moved {
        // Report the *clamped* index so the app mirrors what the engine
        // actually did rather than what was requested.
        Some(to_index) => {
            let _ = ctx.event_tx.send(AudioEvent::MasterPluginMoved {
                instance_id,
                to_index,
            });
        }
        None => {
            let _ = ctx.event_tx.send(AudioEvent::Error(format!(
                "Cannot reorder plugin {} on the master: it is not on the \
                 master chain",
                instance_id
            )));
        }
    }
}

pub(crate) fn handle_set_master_fx_bypass(ctx: &HandlerCtx, bypassed: bool) {
    ctx.shared
        .master_fx_bypassed
        .store(bypassed, Ordering::Relaxed);
    let _ = ctx
        .event_tx
        .send(AudioEvent::MasterFxBypassChanged { bypassed });
}
