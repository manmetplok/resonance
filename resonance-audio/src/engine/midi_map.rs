//! MIDI Learn & hardware-controller mapping on the engine control thread
//! (architecture doc #167 §2 E3, epic #21).
//!
//! The engine owns the **active binding set** and the **learn arm**, and
//! does the one thing only it can do cheaply: look at every message the
//! control-surface port delivers ([`LiveControlEvent`]) and decide what it
//! is for.
//!
//! - **Learn armed** — the first CC, or the first note-*on*, is captured:
//!   `AudioEvent::MidiLearnCaptured { target, source }`, and the arm drops.
//!   A CC is captured as [`CcMode::Absolute`]; relative encoders come from
//!   a controller map, which says so explicitly.
//! - **A bound control** — `AudioEvent::ControlSurfaceMoved { binding,
//!   value }` carries the binding the message matched and its raw 7-bit
//!   value (the CC value, or the note-on velocity). A note-off is dropped:
//!   toggles and triggers fire on the press.
//! - **Anything else** is dropped, so an unmapped knob costs no event.
//!
//! Applying the value is the **app's** job, not this module's. The app
//! holds the model a move has to land in — the track's dB, the plugin's
//! `min..=max`, the undo history, automation write — and already has one
//! message per control that does all of that. A hardware move that went
//! straight into the mixer here would have to be mirrored back into each
//! of those by hand, and the soft-takeover and relative-encoder math need
//! the target's *current* value, which the app is the authority on. So
//! the engine matches and the app applies, through the same messages a
//! mouse drag sends ([`resonance_common::midi_map`] holds the shared
//! mapping math both use).
//!
//! No read-getters: every change to the set is echoed back as
//! `MidiBindingChanged` / `MidiBindingCleared`, so the app's mirror is a
//! projection of events (doc #105).

use indexmap::IndexMap;
use resonance_common::{
    BindingId, CcMode, ControlSource, ControllerMap, MidiBinding, MidiTarget,
};

use crate::midi_hardware::LiveControlEvent;
use crate::types::AudioEvent;

use super::thread::{HandlerCtx, HandlerState};

/// The engine-thread half of the mapping: the active bindings (in the
/// order they were set, so echoes and a controller map's stream arrive in
/// a stable order) and the armed learn target.
#[derive(Debug, Default)]
pub struct ActiveMidiMap {
    bindings: IndexMap<BindingId, MidiBinding>,
    learn: Option<MidiTarget>,
}

impl ActiveMidiMap {
    /// The binding a control message from `(channel, cc)` / `(channel,
    /// note)` drives, if any. A CC binding matches whatever its mode: the
    /// mode says how to read the value, not which control it is.
    fn matching(&self, event: &LiveControlEvent) -> Option<&MidiBinding> {
        self.bindings.values().find(|b| match (b.source, event) {
            (
                ControlSource::Cc { channel, cc, .. },
                LiveControlEvent::Cc {
                    channel: ch, cc: n, ..
                },
            ) => channel == *ch && cc == *n,
            (
                ControlSource::Note { channel, note },
                LiveControlEvent::Note {
                    channel: ch,
                    note: n,
                    ..
                },
            ) => channel == *ch && note == *n,
            _ => false,
        })
    }

    /// Insert or replace `binding`. A physical control drives one target,
    /// so any OTHER binding on the same control is dropped; its id is
    /// returned so the caller can echo the removal.
    fn upsert(&mut self, binding: MidiBinding) -> Vec<BindingId> {
        let same_control = |a: ControlSource, b: ControlSource| match (a, b) {
            (
                ControlSource::Cc { channel, cc, .. },
                ControlSource::Cc {
                    channel: ch, cc: n, ..
                },
            ) => channel == ch && cc == n,
            (a, b) => a == b,
        };
        let displaced: Vec<BindingId> = self
            .bindings
            .values()
            .filter(|b| b.id != binding.id && same_control(b.source, binding.source))
            .map(|b| b.id)
            .collect();
        for id in &displaced {
            self.bindings.shift_remove(id);
        }
        self.bindings.insert(binding.id, binding);
        displaced
    }
}

fn emit(ctx: &HandlerCtx, event: AudioEvent) {
    let _ = ctx.event_tx.send(event);
}

/// Insert or replace a single binding by id, echoing it (and any binding
/// it displaced from the same control).
pub(crate) fn handle_set_midi_binding(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    binding: MidiBinding,
) {
    for id in state.midi_hw.map.upsert(binding) {
        emit(ctx, AudioEvent::MidiBindingCleared { id });
    }
    emit(ctx, AudioEvent::MidiBindingChanged { binding });
}

/// Remove the active binding with this id; a silent no-op when there is
/// none.
pub(crate) fn handle_clear_midi_binding(ctx: &HandlerCtx, state: &mut HandlerState, id: BindingId) {
    if state.midi_hw.map.bindings.shift_remove(&id).is_some() {
        emit(ctx, AudioEvent::MidiBindingCleared { id });
    }
}

/// Replace the whole active set with `map`'s bindings: one
/// `MidiBindingCleared` per binding that goes, then one
/// `MidiBindingChanged` per binding of the map, in map order. Project load
/// and undo use this, as does loading a controller preset.
pub(crate) fn handle_set_controller_map(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    map: ControllerMap,
) {
    handle_clear_all_midi_bindings(ctx, state);
    for binding in map.bindings {
        handle_set_midi_binding(ctx, state, binding);
    }
}

/// Drop every active binding, echoing each removal.
pub(crate) fn handle_clear_all_midi_bindings(ctx: &HandlerCtx, state: &mut HandlerState) {
    let gone: Vec<BindingId> = state.midi_hw.map.bindings.keys().copied().collect();
    state.midi_hw.map.bindings.clear();
    for id in gone {
        emit(ctx, AudioEvent::MidiBindingCleared { id });
    }
}

/// Pick or clear the dedicated control-surface input port. A device that
/// is not plugged in is remembered and opened when it appears (the
/// `ListMidiInputs` poll reconciles it).
pub(crate) fn handle_set_control_surface_input(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    device: Option<String>,
) {
    if let Err(e) = state.midi_hw.control_surface.set_input(device) {
        emit(ctx, AudioEvent::Error(e.into()));
    }
}

/// Arm MIDI Learn for a target, replacing any earlier arm.
pub(crate) fn handle_enter_midi_learn(state: &mut HandlerState, target: MidiTarget) {
    state.midi_hw.map.learn = Some(target);
}

/// Cancel an armed MIDI Learn without capturing anything.
pub(crate) fn handle_cancel_midi_learn(state: &mut HandlerState) {
    state.midi_hw.map.learn = None;
}

/// One drained control-surface message: a learn capture, a bound move, or
/// nothing (see the module docs).
pub(crate) fn handle_control_event(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    event: LiveControlEvent,
) {
    let map = &mut state.midi_hw.map;
    if let Some(target) = map.learn {
        let source = match event {
            LiveControlEvent::Cc { channel, cc, .. } => Some(ControlSource::Cc {
                channel,
                cc,
                mode: CcMode::Absolute,
            }),
            LiveControlEvent::Note {
                channel,
                note,
                velocity,
                ..
            } if velocity > 0 => Some(ControlSource::Note { channel, note }),
            // A release is never what the user meant to learn.
            LiveControlEvent::Note { .. } => None,
        };
        if let Some(source) = source {
            map.learn = None;
            emit(ctx, AudioEvent::MidiLearnCaptured { target, source });
        }
        return;
    }
    let value = match event {
        LiveControlEvent::Cc { value, .. } => value,
        LiveControlEvent::Note { velocity: 0, .. } => return,
        LiveControlEvent::Note { velocity, .. } => velocity,
    };
    if let Some(binding) = map.matching(&event).copied() {
        emit(ctx, AudioEvent::ControlSurfaceMoved { binding, value });
    }
}

/// The input-port names to offer as a control surface, from the latest
/// MIDI input enumeration.
pub(crate) fn report_control_surface_devices(ctx: &HandlerCtx, inputs: Vec<String>) {
    emit(ctx, AudioEvent::ControlSurfaceDevicesChanged { inputs });
}
