//! MIDI Learn and hardware controller mapping command dispatch.

use crate::types::*;

use super::super::{HandlerCtx, HandlerState};
use super::super::super::midi_map;

pub(super) fn dispatch_midi_map(ctx: &HandlerCtx, state: &mut HandlerState, cmd: AudioCommand) {
    match cmd {
        AudioCommand::SetMidiBinding { binding } => {
            midi_map::handle_set_midi_binding(ctx, state, binding)
        }
        AudioCommand::ClearMidiBinding { id } => {
            midi_map::handle_clear_midi_binding(ctx, state, id)
        }
        AudioCommand::SetControllerMap { map } => {
            midi_map::handle_set_controller_map(ctx, state, map)
        }
        AudioCommand::ClearAllMidiBindings => {
            midi_map::handle_clear_all_midi_bindings(ctx, state)
        }
        AudioCommand::SetControlSurfaceInput { device } => {
            midi_map::handle_set_control_surface_input(ctx, state, device)
        }
        AudioCommand::EnterMidiLearn { target } => midi_map::handle_enter_midi_learn(state, target),
        AudioCommand::CancelMidiLearn => midi_map::handle_cancel_midi_learn(state),
        _ => unreachable!("dispatch_midi_map: unexpected command"),
    }
}
