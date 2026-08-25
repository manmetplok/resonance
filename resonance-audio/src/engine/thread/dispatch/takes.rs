//! Take-lane command dispatch: comp edits, active-take selection, take /
//! lane removal, and the project-load rehydration of the take-group store
//! (epic #15, doc #165).

use crate::types::*;

use super::super::super::takes;
use super::super::{HandlerCtx, HandlerState};

pub(super) fn dispatch_takes(ctx: &HandlerCtx, state: &mut HandlerState, cmd: AudioCommand) {
    match cmd {
        AudioCommand::SetTakeComp { group_id, segments } => {
            takes::handle_set_take_comp(ctx, state, group_id, segments)
        }
        AudioCommand::SetActiveTake { group_id, take_id } => {
            takes::handle_set_active_take(ctx, state, group_id, take_id)
        }
        AudioCommand::RemoveTake { group_id, take_id } => {
            takes::handle_remove_take(ctx, state, group_id, take_id)
        }
        AudioCommand::RemoveTakeGroup { group_id } => {
            takes::handle_remove_take_group(ctx, state, group_id)
        }
        AudioCommand::RestoreTakeGroups { groups } => {
            takes::handle_restore_take_groups(ctx, state, groups)
        }
        // The audio half of the same restore: the groups name clip ids,
        // this puts the clips behind them back in the engine's list.
        // Routed here rather than with the timeline clip commands because
        // what makes it a different command from `LoadClipFromWav` is a
        // take-lane rule, not a clip-loading one (ba todo #1402).
        AudioCommand::LoadTakeClipFromWav {
            clip_id,
            track_id,
            start_sample,
            path,
            name,
        } => crate::engine::clips::handle_load_take_clip_from_wav(
            ctx,
            state,
            clip_id,
            track_id,
            start_sample,
            path,
            name,
        ),
        _ => unreachable!("dispatch_takes: unexpected command"),
    }
}
