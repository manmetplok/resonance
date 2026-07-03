//! MIDI Learn and hardware controller mapping command dispatch.

use crate::types::*;

use super::super::HandlerCtx;
use super::super::super::midi_map;

pub(super) fn dispatch_midi_map(ctx: &HandlerCtx, cmd: AudioCommand) {
    match cmd {
        AudioCommand::SetMidiBinding { binding } => {
            midi_map::handle_set_midi_binding(ctx, binding)
        }
        AudioCommand::ClearMidiBinding { id } => midi_map::handle_clear_midi_binding(ctx, id),
        AudioCommand::SetControllerMap { map } => midi_map::handle_set_controller_map(ctx, map),
        AudioCommand::ClearAllMidiBindings => midi_map::handle_clear_all_midi_bindings(ctx),
        AudioCommand::SetControlSurfaceInput { device } => {
            midi_map::handle_set_control_surface_input(ctx, device)
        }
        AudioCommand::EnterMidiLearn { target } => midi_map::handle_enter_midi_learn(ctx, target),
        AudioCommand::CancelMidiLearn => midi_map::handle_cancel_midi_learn(ctx),
        _ => unreachable!("dispatch_midi_map: unexpected command"),
    }
}
