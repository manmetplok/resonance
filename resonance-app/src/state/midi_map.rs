//! MIDI Learn & hardware-controller mapping — GUI-side state (architecture
//! doc #167 §3 A1, epic #21).
//!
//! A pure projection of the engine's active binding set: the engine owns the
//! bindings and echoes every change back as `MidiBindingChanged` /
//! `MidiBindingCleared` events (doc #105, no read-getters), and this state is
//! rebuilt from those events alone — including after project-load replay. The
//! mapping math itself lives in [`resonance_common::midi_map`]; this type only
//! holds what the view needs to draw and edit the mappings.

use std::collections::HashMap;

use resonance_common::{BindingId, ControlSource, MidiBinding, MidiTarget};

/// GUI-side mirror of the controller mapping. Held as a sub-struct on
/// `Resonance` so handlers that only touch mapping take `&mut MidiMapState`.
#[derive(Debug, Clone, Default)]
pub struct MidiMapState {
    /// Active bindings keyed by id. Reconstructed purely from
    /// `MidiBindingChanged` (upsert) / `MidiBindingCleared` (remove).
    pub bindings: HashMap<BindingId, MidiBinding>,
    /// Reverse index: which binding listens to a given physical control.
    /// Lets an incoming control resolve to its binding (and a learn /
    /// edit flow detect a source conflict) without scanning `bindings`.
    /// A `ControlSource` drives at most one binding.
    pub source_index: HashMap<ControlSource, BindingId>,
    /// The target armed for MIDI Learn, or `None` when not learning. Set
    /// when the user arms learn (an update-handler concern); cleared here
    /// when the engine reports the captured control (`MidiLearnCaptured`).
    pub learn_target: Option<MidiTarget>,
    /// Latest normalized value a hardware move drove each target to
    /// (`ControlSurfaceParamChanged`), so the view can tint / animate the
    /// matching on-screen control. A transient display hint, not authority
    /// over the real parameter value.
    pub live_values: HashMap<MidiTarget, f32>,
    /// Control-surface MIDI input port names the engine currently offers
    /// (`ControlSurfaceDevicesChanged`), for the device picker.
    pub available_inputs: Vec<String>,
    /// Monotonic id allocator for newly-learned bindings, kept ahead of
    /// every id seen from the engine so app-allocated ids never collide
    /// with project-loaded / preset ones.
    next_id: u64,
}

impl MidiMapState {
    /// Insert or replace a binding (mirror of `MidiBindingChanged`),
    /// keeping `source_index` and the id allocator consistent.
    pub fn upsert(&mut self, binding: MidiBinding) {
        // If this id previously listened to a different source, drop the
        // stale reverse-index entry before re-pointing it.
        if let Some(old) = self.bindings.get(&binding.id) {
            if old.source != binding.source {
                self.source_index.remove(&old.source);
            }
        }
        self.source_index.insert(binding.source, binding.id);
        self.next_id = self.next_id.max(binding.id.0 + 1);
        self.bindings.insert(binding.id, binding);
    }

    /// Remove a binding by id (mirror of `MidiBindingCleared`). A no-op if
    /// no such binding is active.
    pub fn clear(&mut self, id: BindingId) {
        if let Some(b) = self.bindings.remove(&id) {
            // Only drop the reverse-index entry if it still points at this
            // binding — a later upsert may have re-claimed the source.
            if self.source_index.get(&b.source) == Some(&id) {
                self.source_index.remove(&b.source);
            }
        }
    }

    /// Allocate a fresh, unused binding id for a newly-learned control.
    pub fn alloc_id(&mut self) -> BindingId {
        let id = BindingId(self.next_id);
        self.next_id += 1;
        id
    }
}
