//! Transport command dispatch: Play / Pause / Stop / Seek / BPM / loop.

use crate::types::*;

use super::super::{HandlerCtx, HandlerState};
use super::super::super::transport;

pub(super) fn dispatch_transport(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    cmd: AudioCommand,
) {
    match cmd {
        AudioCommand::Play => transport::handle_play(ctx, state),
        AudioCommand::Record { precount_bars } => {
            transport::handle_record(ctx, state, precount_bars)
        }
        AudioCommand::Pause => transport::handle_pause(ctx, state),
        AudioCommand::Stop => transport::handle_stop(ctx, state),
        AudioCommand::SeekTo(pos) => transport::handle_seek_to(ctx, state, pos),
        AudioCommand::SetBpm { bpm } => transport::handle_set_bpm(ctx, bpm),
        AudioCommand::SetTempoEvents { tempo, signature } => {
            transport::handle_set_tempo_events(ctx, tempo, signature)
        }
        AudioCommand::SetTimeSignature {
            numerator,
            denominator,
        } => transport::handle_set_time_signature(ctx, numerator, denominator),
        AudioCommand::SetMetronomeEnabled { enabled } => {
            transport::handle_set_metronome_enabled(ctx, enabled)
        }
        AudioCommand::SetLoopRange {
            enabled,
            loop_in,
            loop_out,
        } => transport::handle_set_loop_range(ctx, state, enabled, loop_in, loop_out),
        AudioCommand::SetLoopRecordMode(on) => transport::handle_set_loop_record_mode(state, on),
        _ => unreachable!("dispatch_transport: unexpected command"),
    }
}
