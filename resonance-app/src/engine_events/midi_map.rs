//! App-side handling of the MIDI Learn / hardware control-surface
//! mapping engine events (architecture doc #167 §3 A1, epic #21).
//!
//! The binding echoes fold into `MidiMapState` (idempotently: the app has
//! already applied the edit they confirm). A learn capture and a hardware
//! move are user edits that happen to arrive from the engine, so they go
//! through the full update path — `MidiMapMessage::Bind`, or the moved
//! control's own message — to be undoable like any other edit.

use iced::Task;
use resonance_common::{BindingId, ControlSource, MidiBinding, MidiTarget};

use crate::message::{Message, MidiMapMessage};
use crate::Resonance;

/// Mirror `MidiBindingChanged`: insert or replace the binding.
pub(super) fn binding_changed(r: &mut Resonance, binding: MidiBinding) {
    r.devices.midi_map.upsert(binding);
}

/// Mirror `MidiBindingCleared`: drop the binding from the active set (echo
/// of `ClearMidiBinding`, or one per binding of `ClearAllMidiBindings` /
/// a `SetControllerMap` replacing the set).
pub(super) fn binding_cleared(r: &mut Resonance, id: BindingId) {
    r.devices.midi_map.clear(id);
}

/// Handle `MidiLearnCaptured`: bind the captured control to the armed
/// target, as one undoable edit.
pub(super) fn learn_captured(
    r: &mut Resonance,
    target: MidiTarget,
    source: ControlSource,
) -> Task<Message> {
    r.update(Message::MidiMap(MidiMapMessage::Bind { target, source }))
}

/// Handle `ControlSurfaceMoved`: apply the move to its target.
pub(super) fn moved(r: &mut Resonance, binding: MidiBinding, value: u8) -> Task<Message> {
    crate::update::midi_map::hardware_moved(r, binding, value)
}

/// Mirror `ControlSurfaceDevicesChanged`: refresh the list of available
/// control-surface MIDI input ports for the device picker.
pub(super) fn devices_changed(r: &mut Resonance, inputs: Vec<String>) {
    r.devices.midi_map.available_inputs = inputs;
}
