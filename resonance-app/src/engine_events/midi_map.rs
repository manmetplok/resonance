//! App-side projection of the MIDI Learn / hardware control-surface mapping
//! engine events into `MidiMapState` (architecture doc #167 §3 A1, epic #21).
//!
//! Pure projection (doc #105): every handler here consumes an `AudioEvent`
//! and folds it into app state — no read-getters on the engine. The one
//! command this module sends is `SetMidiBinding`, emitted when a learn
//! capture completes so the engine adopts the new binding (and echoes it
//! back via `MidiBindingChanged`, idempotently re-applying it).

use resonance_audio::types::*;
use resonance_common::{BindingId, ControlSource, MidiBinding, MidiTarget};

use crate::Resonance;

/// Mirror `MidiBindingChanged`: insert or replace the binding. This is the
/// single entry point through which the active set is rebuilt — including
/// the per-binding stream emitted by `SetControllerMap` / project-load
/// replay, which simply arrives as a sequence of these.
pub(super) fn binding_changed(r: &mut Resonance, binding: MidiBinding) {
    r.midi_map.upsert(binding);
}

/// Mirror `MidiBindingCleared`: drop the binding from the active set (echo
/// of `ClearMidiBinding`, or one per binding of `ClearAllMidiBindings`).
pub(super) fn binding_cleared(r: &mut Resonance, id: BindingId) {
    r.midi_map.clear(id);
}

/// Handle `MidiLearnCaptured`: the engine captured `source` for the armed
/// target. Record a binding with default range / mode / takeover, send
/// `SetMidiBinding` so the engine adopts it (and echoes it back), and leave
/// learn mode.
pub(super) fn learn_captured(r: &mut Resonance, target: MidiTarget, source: ControlSource) {
    let id = r.midi_map.alloc_id();
    let binding = MidiBinding::new(id, source, target);
    r.midi_map.upsert(binding);
    let _ = r.engine.send(AudioCommand::SetMidiBinding { binding });
    r.midi_map.learn_target = None;
}

/// Handle `ControlSurfaceParamChanged`: a hardware move drove `target` to
/// `value_norm`. Stash it as a transient display hint so the view can tint
/// the matching on-screen control.
pub(super) fn param_changed(r: &mut Resonance, target: MidiTarget, value_norm: f32) {
    r.midi_map.live_values.insert(target, value_norm);
}

/// Mirror `ControlSurfaceDevicesChanged`: refresh the list of available
/// control-surface MIDI input ports for the device picker.
pub(super) fn devices_changed(r: &mut Resonance, inputs: Vec<String>) {
    r.midi_map.available_inputs = inputs;
}
