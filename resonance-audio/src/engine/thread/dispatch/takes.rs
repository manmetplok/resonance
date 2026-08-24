//! Take-lane command dispatch: comp edits, active-take selection, and the
//! project-load rehydration of the take-group store (epic #15, doc #165).

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
        AudioCommand::RestoreTakeGroups { groups } => {
            takes::handle_restore_take_groups(ctx, state, groups)
        }
        _ => unreachable!("dispatch_takes: unexpected command"),
    }
}
