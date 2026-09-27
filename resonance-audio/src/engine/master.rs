//! Master-bus plugin handlers. The master bus owns an insert chain that
//! runs after every track and bus has been summed into the output, just
//! before the master volume + clip + peak pass.
//!
//! Mechanically these handlers are a trimmed clone of the bus plugin
//! handlers in `busses.rs` — same bundle lookup, same instance creation,
//! same retry-on-lock pattern via `cmd_tx_retry`. The only difference is
//! that the plugin list lives on `MasterBus` instead of a keyed `Bus`.

use std::path::Path;

use crate::types::*;

use super::plugins::{
    ensure_bundle, reject_if_plugin_id_in_use, report_plugin_load_failure, resolve_plugin_id,
};
use super::thread::{HandlerCtx, HandlerState};

pub(crate) fn handle_add_plugin_to_master(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
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
            report_plugin_load_failure(ctx, Some(id), &clap_plugin_id, &clap_file_path, reason.to_string());
            return;
        }
    };
    let actual_plugin_id =
        match resolve_plugin_id(&state.bundles[bundle_idx], clap_plugin_id.clone()) {
            Ok(resolved) => resolved,
            Err(reason) => {
                report_plugin_load_failure(ctx, Some(id), &clap_plugin_id, &clap_file_path, reason.to_string());
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
            ctx.shared
                .edit_master(|master| master.plugin_ids.push(instance_id));
            let _ = ctx.event_tx.send(AudioEvent::MasterPluginAdded {
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

pub(crate) fn handle_remove_plugin_from_master(ctx: &HandlerCtx, instance_id: PluginInstanceId) {
    if ctx.shared.graph.load().master.plugin_ids.contains(&instance_id) {
        ctx.shared
            .edit_master(|master| master.plugin_ids.retain(|&id| id != instance_id));
    }
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
    // A plugin that is not on the chain publishes nothing.
    let on_chain = ctx.shared.graph.load().master.plugin_ids.contains(&instance_id);
    let moved = on_chain
        .then(|| {
            ctx.shared
                .edit_master(|master| master.move_plugin(instance_id, to_index))
        })
        .flatten();
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
            let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::not_found(format!(
                "Cannot reorder plugin {} on the master: it is not on the \
                 master chain",
                instance_id
            ))));
        }
    }
}

pub(crate) fn handle_set_master_fx_bypass(ctx: &HandlerCtx, bypassed: bool) {
    super::plugins::apply_bypass_request(ctx.shared, &ctx.shared.master_fx_bypass, bypassed);
    let _ = ctx
        .event_tx
        .send(AudioEvent::MasterFxBypassChanged { bypassed });
}
