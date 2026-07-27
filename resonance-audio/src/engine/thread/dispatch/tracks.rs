//! Track command dispatch: add/remove/volume/pan/mute/arm/freeze/mono/monitor.

use crate::types::*;

use super::super::{HandlerCtx, HandlerState};
use super::super::super::tracks;

pub(super) fn dispatch_tracks(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    cmd: AudioCommand,
) {
    match cmd {
        AudioCommand::SetTrackVolume { track_id, volume } => {
            tracks::handle_set_track_volume(ctx, track_id, volume)
        }
        AudioCommand::SetTrackPan { track_id, pan } => {
            tracks::handle_set_track_pan(ctx, track_id, pan)
        }
        AudioCommand::SetTrackMute { track_id, muted } => {
            tracks::handle_set_track_mute(ctx, track_id, muted)
        }
        AudioCommand::SetMasterVolume { volume } => tracks::handle_set_master_volume(ctx, volume),
        AudioCommand::SetTrackSolo { track_id, soloed } => {
            tracks::handle_set_track_solo(ctx, track_id, soloed)
        }
        AudioCommand::AddTrack { id_hint, name } => {
            tracks::handle_add_track(ctx, state, id_hint, name)
        }
        AudioCommand::CreateSubTrack {
            sub_id,
            parent_track_id,
            output_port_index,
            name,
        } => tracks::handle_create_sub_track(
            ctx,
            state,
            sub_id,
            parent_track_id,
            output_port_index,
            name,
        ),
        AudioCommand::RemoveTrack { track_id } => {
            tracks::handle_remove_track(ctx, state, track_id)
        }
        AudioCommand::SetTrackRecordArm { track_id, armed } => {
            tracks::handle_set_track_record_arm(ctx, track_id, armed)
        }
        AudioCommand::SetTrackMono { track_id, mono } => {
            tracks::handle_set_track_mono(ctx, state, track_id, mono)
        }
        AudioCommand::SetTrackMonitor { track_id, enabled } => {
            tracks::handle_set_track_monitor(ctx, state, track_id, enabled)
        }
        AudioCommand::SetTrackInputDevice {
            track_id,
            device_name,
        } => tracks::handle_set_track_input_device(ctx, state, track_id, device_name),
        AudioCommand::SetTrackInputPort {
            track_id,
            port_index,
        } => tracks::handle_set_track_input_port(ctx, state, track_id, port_index),
        AudioCommand::ListInputDevices => tracks::handle_list_input_devices(ctx),
        AudioCommand::ClearAll => tracks::handle_clear_all(ctx, state),
        AudioCommand::SetTrackFrozenSource { track_id, source } => {
            tracks::handle_set_track_frozen_source(ctx, track_id, source)
        }
        AudioCommand::UnfreezeTrack { track_id } => {
            tracks::handle_unfreeze_track(ctx, track_id)
        }
        AudioCommand::SetTrackFxBypass { track_id, bypassed } => {
            tracks::handle_set_track_fx_bypass(ctx, track_id, bypassed)
        }
        _ => unreachable!("dispatch_tracks: unexpected command"),
    }
}
