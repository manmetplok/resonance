//! Plugin command dispatch: chain edits (any owner), param/editor/state/scan.

use crate::types::*;

use super::super::{HandlerCtx, HandlerState};
use super::super::super::{chain, plugins, scan};

pub(super) fn dispatch_plugins(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    cmd: AudioCommand,
) {
    match cmd {
        AudioCommand::AddPlugin {
            owner,
            clap_file_path,
            clap_plugin_id,
            id,
        } => chain::handle_add_plugin(ctx, state, owner, clap_file_path, clap_plugin_id, id),
        AudioCommand::RemovePlugin { owner, instance_id } => {
            chain::handle_remove_plugin(ctx, state, owner, instance_id)
        }
        AudioCommand::MovePlugin {
            owner,
            instance_id,
            to_index,
        } => chain::handle_move_plugin(ctx, owner, instance_id, to_index),
        AudioCommand::SetFxBypass { owner, bypassed } => {
            chain::handle_set_fx_bypass(ctx, owner, bypassed)
        }
        AudioCommand::ScanPlugins => {
            scan::scan_plugins(ctx.shared, &ctx.tracks(), &mut state.bundles, ctx.event_tx)
        }
        // Additive by construction: it never touches the plugin map, which
        // is what lets it run with instances live (ba todo #1307).
        AudioCommand::RescanPlugins => scan::rescan_plugins(&mut state.bundles, ctx.event_tx),
        AudioCommand::SetPluginParam {
            instance_id,
            param_id,
            value,
        } => plugins::handle_set_plugin_param(ctx, instance_id, param_id, value),
        AudioCommand::ResolvePluginParamText {
            instance_id,
            param_id,
            text,
            token,
        } => plugins::handle_resolve_param_text(ctx, instance_id, param_id, text, token),
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
        AudioCommand::SavePluginPresetState { instance_id } => {
            plugins::handle_save_plugin_preset_state(ctx, instance_id)
        }
        AudioCommand::LoadPluginPresetState {
            instance_id,
            data,
            capture,
        } => plugins::handle_load_plugin_preset_state(ctx, instance_id, data, capture),
        AudioCommand::LoadPluginPresetFromLocation {
            instance_id,
            location,
            load_key,
            capture,
        } => plugins::handle_load_plugin_preset_from_location(
            ctx,
            instance_id,
            location,
            load_key,
            capture,
        ),
        AudioCommand::SetPluginPresetIgnoredParams {
            instance_id,
            clap_ids,
        } => plugins::handle_set_plugin_preset_ignored_params(ctx, instance_id, clap_ids),
        AudioCommand::SaveAllPluginStates => plugins::handle_save_all_plugin_states(ctx),
        _ => unreachable!("dispatch_plugins: unexpected command"),
    }
}
