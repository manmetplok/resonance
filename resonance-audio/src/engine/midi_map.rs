//! MIDI Learn & hardware-controller mapping — command handlers
//! (architecture doc #167 §2, epic #21).
//!
//! This module owns the engine-thread side of the control-surface mapping
//! command/event boundary (doc #105). **This is the E2 plumbing slice**: the
//! command variants, the matching events, and the handler entry points wired
//! into [`super::thread::dispatch`] exist and compile, but the handlers are
//! intentionally stubs.
//!
//! The real behaviour — maintaining the active binding set, draining
//! `LiveControlEvent`s, soft-takeover, and emitting `MidiBindingChanged` /
//! `MidiLearnCaptured` / `ControlSurfaceParamChanged` — lands in **E3**
//! (binding application on the engine control thread). Keeping the handlers
//! here, named and routed, gives E3 a single place to grow into without
//! re-touching the command/event enums or the dispatch table.
//!
//! No read-getters: app state is rebuilt purely from the events these
//! handlers will emit, matching the rest of the engine boundary.

use resonance_common::{BindingId, ControllerMap, MidiBinding, MidiTarget};

use super::thread::HandlerCtx;

/// Insert or replace a single binding by id.
///
/// Stub (E2): the active binding set lives on the engine thread and is
/// introduced with the drain/application logic in E3, which will store the
/// binding and echo it back via `AudioEvent::MidiBindingChanged`.
pub(crate) fn handle_set_midi_binding(_ctx: &HandlerCtx, _binding: MidiBinding) {
    // TODO(E3): upsert into the active binding set + emit MidiBindingChanged.
}

/// Remove the active binding with this id.
///
/// Stub (E2): E3 removes it from the active set and emits
/// `AudioEvent::MidiBindingCleared`.
pub(crate) fn handle_clear_midi_binding(_ctx: &HandlerCtx, _id: BindingId) {
    // TODO(E3): remove from the active set + emit MidiBindingCleared.
}

/// Replace the entire active binding set from a controller-map preset.
///
/// Stub (E2): E3 swaps the active set and emits one
/// `AudioEvent::MidiBindingChanged` per resulting binding so the app can
/// rebuild its state from events alone.
pub(crate) fn handle_set_controller_map(_ctx: &HandlerCtx, _map: ControllerMap) {
    // TODO(E3): replace the active set + emit MidiBindingChanged per binding.
}

/// Drop every active binding.
///
/// Stub (E2): E3 clears the active set and emits
/// `AudioEvent::MidiBindingCleared` per removed binding.
pub(crate) fn handle_clear_all_midi_bindings(_ctx: &HandlerCtx) {
    // TODO(E3): clear the active set + emit MidiBindingCleared per binding.
}

/// Pick or clear the dedicated control-surface input port.
///
/// Stub (E2): E1 adds the control-surface input thread; E3 opens/closes the
/// selected port here and emits `AudioEvent::ControlSurfaceDevicesChanged`.
pub(crate) fn handle_set_control_surface_input(_ctx: &HandlerCtx, _device: Option<String>) {
    // TODO(E3): open/close the control-surface input port.
}

/// Arm MIDI Learn for a target.
///
/// Stub (E2): E3 stashes the armed target so the next qualifying control
/// message is reported via `AudioEvent::MidiLearnCaptured` instead of applied.
pub(crate) fn handle_enter_midi_learn(_ctx: &HandlerCtx, _target: MidiTarget) {
    // TODO(E3): arm learn mode for `target`.
}

/// Cancel an armed MIDI Learn.
///
/// Stub (E2): E3 clears the armed target without capturing anything.
pub(crate) fn handle_cancel_midi_learn(_ctx: &HandlerCtx) {
    // TODO(E3): disarm learn mode.
}
