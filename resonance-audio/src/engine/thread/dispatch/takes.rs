//! Take-lane command dispatch: comp edits and active-take selection
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
        _ => unreachable!("dispatch_takes: unexpected command"),
    }
}
