//! Plugin command dispatch: add/remove/param/editor/state/scan.

use crate::types::*;

use super::super::{HandlerCtx, HandlerState};
use super::super::super::{plugins, scan};

pub(super) fn dispatch_plugins(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    cmd: AudioCommand,
) {
    match cmd {
        AudioCommand::AddPlugin {
            track_id,
            clap_file_path,
            clap_plugin_id,
            id,
        } => plugins::handle_add_plugin(
            ctx,
            state,
            track_id,
            clap_file_path,
            clap_plugin_id,
            id,
        ),
        AudioCommand::RemovePlugin {
            track_id,
            instance_id,
        } => {
            plugins::handle_remove_plugin(ctx, track_id, instance_id);
            crate::engine::sidechain::drop_plugin_route(ctx, state, instance_id);
        }
        AudioCommand::MovePlugin {
            track_id,
            instance_id,
            to_index,
        } => plugins::handle_move_plugin(ctx, track_id, instance_id, to_index),
        AudioCommand::ScanPlugins => {
            scan::scan_plugins(ctx.plugins, ctx.tracks, &mut state.bundles, ctx.event_tx)
        }
        // Additive by construction: it never touches `ctx.plugins`, which
        // is what lets it run with instances live (ba todo #1307).
        AudioCommand::RescanPlugins => scan::rescan_plugins(&mut state.bundles, ctx.event_tx),
        AudioCommand::SetPluginParam {
            instance_id,
            param_id,
            value,
        } => plugins::handle_set_plugin_param(ctx, instance_id, param_id, value),
        AudioCommand::SetPluginBypass {
            instance_id,
            bypassed,
        } => plugins::handle_set_plugin_bypass(ctx, instance_id, bypassed),
        AudioCommand::OpenPluginEditor { instance_id } => {
            plugins::handle_open_plugin_editor(ctx, instance_id)
        }
        AudioCommand::ClosePluginEditor { instance_id } => {
            plugins::handle_close_plugin_editor(ctx, instance_id)
        }
        AudioCommand::SavePluginState { instance_id } => {
            plugins::handle_save_plugin_state(ctx, instance_id)
        }
        AudioCommand::LoadPluginState { instance_id, data } => {
            plugins::handle_load_plugin_state(ctx, instance_id, data)
        }
        AudioCommand::SaveAllPluginStates => plugins::handle_save_all_plugin_states(ctx),
        _ => unreachable!("dispatch_plugins: unexpected command"),
    }
}
