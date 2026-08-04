//! Bus, aux-send, and master FX chain command dispatch.

use crate::types::*;

use super::super::{HandlerCtx, HandlerState};
use super::super::super::{busses, master};

pub(super) fn dispatch_busses(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    cmd: AudioCommand,
) {
    match cmd {
        AudioCommand::AddBus { id_hint, name } => {
            busses::handle_add_bus(ctx, state, id_hint, name)
        }
        AudioCommand::RemoveBus { bus_id } => {
            busses::handle_remove_bus(ctx, bus_id);
            crate::engine::sidechain::drop_source_routes(
                ctx,
                state,
                crate::types::SendSource::Bus(bus_id),
            );
        }
        AudioCommand::SetBusVolume { bus_id, volume } => {
            busses::handle_set_bus_volume(ctx, bus_id, volume)
        }
        AudioCommand::SetBusPan { bus_id, pan } => busses::handle_set_bus_pan(ctx, bus_id, pan),
        AudioCommand::SetBusMute { bus_id, muted } => {
            busses::handle_set_bus_mute(ctx, bus_id, muted)
        }
        AudioCommand::SetBusName { bus_id, name } => {
            busses::handle_set_bus_name(ctx, bus_id, name)
        }
        AudioCommand::SetTrackOutput { track_id, output } => {
            busses::handle_set_track_output(ctx, track_id, output)
        }
        AudioCommand::AddPluginToBus {
            bus_id,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        } => busses::handle_add_plugin_to_bus(
            ctx,
            state,
            bus_id,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        ),
        AudioCommand::RemovePluginFromBus {
            bus_id,
            instance_id,
        } => busses::handle_remove_plugin_from_bus(ctx, bus_id, instance_id),
        AudioCommand::MovePluginInBus {
            bus_id,
            instance_id,
            to_index,
        } => busses::handle_move_plugin_in_bus(ctx, bus_id, instance_id, to_index),
        AudioCommand::SetBusRole { bus_id, is_return } => {
            busses::handle_set_bus_role(ctx, bus_id, is_return)
        }
        AudioCommand::SetAuxSend {
            id_hint,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        } => busses::handle_set_aux_send(
            ctx, state, id_hint, source, dest, level_db, pre_fader, enabled,
        ),
        AudioCommand::RemoveAuxSend { send_id } => {
            busses::handle_remove_aux_send(ctx, state, send_id)
        }
        AudioCommand::AddPluginToMaster {
            clap_file_path,
            clap_plugin_id,
            id_hint,
        } => master::handle_add_plugin_to_master(
            ctx,
            state,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        ),
        AudioCommand::RemovePluginFromMaster { instance_id } => {
            master::handle_remove_plugin_from_master(ctx, instance_id)
        }
        AudioCommand::MovePluginInMaster {
            instance_id,
            to_index,
        } => master::handle_move_plugin_in_master(ctx, instance_id, to_index),
        AudioCommand::SetBusFxBypass { bus_id, bypassed } => {
            busses::handle_set_bus_fx_bypass(ctx, bus_id, bypassed)
        }
        AudioCommand::SetMasterFxBypass { bypassed } => {
            master::handle_set_master_fx_bypass(ctx, bypassed)
        }
        _ => unreachable!("dispatch_busses: unexpected command"),
    }
}
