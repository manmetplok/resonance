//! Audition preview command dispatch.

use crate::types::*;

use super::super::HandlerCtx;
use super::super::super::audition;

pub(super) fn dispatch_audition(ctx: &HandlerCtx, cmd: AudioCommand) {
    match cmd {
        AudioCommand::AuditionFile { path, start_frame } => {
            audition::handle_audition_file(ctx, path, start_frame)
        }
        AudioCommand::StopAudition => {
            if audition::stop_audition_in_place(ctx.shared) {
                let _ = ctx.event_tx.send(AudioEvent::AuditionStopped);
            }
        }
        AudioCommand::SetAuditionOptions {
            loop_enabled,
            sync_to_tempo,
        } => {
            let bpm = ctx.tempo_map.load().bpm as f64;
            audition::set_audition_options_in_place(ctx.shared, bpm, loop_enabled, sync_to_tempo);
        }
        _ => unreachable!("dispatch_audition: unexpected command"),
    }
}
