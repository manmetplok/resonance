//! Bus and aux-send command dispatch.

use crate::types::*;

use super::super::{HandlerCtx, HandlerState};
use super::super::super::busses;

pub(super) fn dispatch_busses(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    cmd: AudioCommand,
) {
    match cmd {
        AudioCommand::AddBus { id, name } => busses::handle_add_bus(ctx, id, name),
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
        AudioCommand::SetBusRole { bus_id, is_return } => {
            busses::handle_set_bus_role(ctx, bus_id, is_return)
        }
        AudioCommand::AddAuxSend {
            id,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        } => busses::handle_add_aux_send(
            ctx, state, id, source, dest, level_db, pre_fader, enabled,
        ),
        AudioCommand::SetAuxSend {
            id,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        } => busses::handle_set_aux_send(
            ctx, state, id, source, dest, level_db, pre_fader, enabled,
        ),
        AudioCommand::RemoveAuxSend { send_id } => {
            busses::handle_remove_aux_send(ctx, state, send_id)
        }
        _ => unreachable!("dispatch_busses: unexpected command"),
    }
}
