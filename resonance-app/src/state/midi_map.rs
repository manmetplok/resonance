//! MIDI Learn & hardware-controller mapping — GUI-side state (architecture
//! doc #167 §3 A1, epic #21).
//!
//! The bindings here are the project's (they persist in `project.json` as
//! `midi_bindings` and are undoable); the engine holds the live copy it
//! matches control-surface messages against. Every edit updates this
//! mirror at once and sends the engine the matching command, whose
//! `MidiBindingChanged` / `MidiBindingCleared` echoes re-apply the same
//! change (idempotently) — so a snapshot taken right after an edit, or a
//! hermetic test with no engine, sees the edit. The mapping math lives in
//! [`resonance_common::midi_map`].

use std::collections::HashMap;

use resonance_common::{BindingId, ControlSource, ControllerMap, MidiBinding, MidiTarget};

/// GUI-side mirror of the controller mapping. Held as a sub-struct on
/// [`state::DeviceState`](crate::state::DeviceState) (A-12f) so handlers
/// that only touch mapping take `&mut MidiMapState`.
#[derive(Debug, Clone, Default)]
pub struct MidiMapState {
    /// Active bindings keyed by id.
    pub bindings: HashMap<BindingId, MidiBinding>,
    /// Reverse index: which binding listens to a given physical control.
    /// A `ControlSource` drives at most one binding.
    pub source_index: HashMap<ControlSource, BindingId>,
    /// The target armed for MIDI Learn, or `None` when not learning. Set
    /// when the user arms learn; cleared by Cancel / Esc and when the
    /// engine reports the captured control (`MidiLearnCaptured`).
    pub learn_target: Option<MidiTarget>,
    /// Control-surface MIDI input port names the engine currently offers
    /// (`ControlSurfaceDevicesChanged`), for the device picker.
    pub available_inputs: Vec<String>,
    /// The open right-click "MIDI" menu, if any.
    pub menu: Option<MidiMenuState>,
    /// The controller-map presets on disk (`controller_maps.json`), read
    /// when Settings opens on its MIDI page.
    pub saved_maps: Vec<ControllerMap>,
    /// The Settings → MIDI "Save as map" name field.
    pub map_name: String,
    /// Monotonic id allocator for newly-learned bindings, kept ahead of
    /// every id seen so app-allocated ids never collide with
    /// project-loaded / preset ones.
    next_id: u64,
}

/// The right-click menu on a mappable control: which target it is for and
/// where it opened, in window coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MidiMenuState {
    pub target: MidiTarget,
    pub x: f32,
    pub y: f32,
}

/// The physical control a source names, without how its value is read: a
/// CC is the same knob whether it is read absolute or relative.
pub fn same_control(a: ControlSource, b: ControlSource) -> bool {
    match (a, b) {
        (
            ControlSource::Cc { channel, cc, .. },
            ControlSource::Cc {
                channel: ch, cc: n, ..
            },
        ) => channel == ch && cc == n,
        (a, b) => a == b,
    }
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

    /// Replace the whole set (a project load, an undo, a controller map).
    pub fn replace_all(&mut self, bindings: impl IntoIterator<Item = MidiBinding>) {
        self.bindings.clear();
        self.source_index.clear();
        for b in bindings {
            self.upsert(b);
        }
    }

    /// Allocate a fresh, unused binding id for a newly-learned control.
    pub fn alloc_id(&mut self) -> BindingId {
        let id = BindingId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Every binding, ordered by id — the order the project file, the
    /// bindings list and `midi_map.bindings` use.
    pub fn sorted(&self) -> Vec<MidiBinding> {
        let mut v: Vec<MidiBinding> = self.bindings.values().copied().collect();
        v.sort_by_key(|b| b.id);
        v
    }

    /// The bindings driving `target`, ordered by id.
    pub fn for_target(&self, target: MidiTarget) -> Vec<MidiBinding> {
        let mut v: Vec<MidiBinding> = self
            .bindings
            .values()
            .filter(|b| b.target == target)
            .copied()
            .collect();
        v.sort_by_key(|b| b.id);
        v
    }

    /// The ids of the bindings on the same physical control as `source`.
    pub fn on_control(&self, source: ControlSource) -> Vec<BindingId> {
        self.bindings
            .values()
            .filter(|b| same_control(b.source, source))
            .map(|b| b.id)
            .collect()
    }
}

/// A binding's control as the UI names it: `CC 7 · ch 1`, `Note C3 · ch 10`
/// (channels 1-based, as hardware labels them).
pub fn source_label(source: ControlSource) -> String {
    match source {
        ControlSource::Cc { channel, cc, mode } => {
            let rel = match mode {
                resonance_common::CcMode::Absolute => "",
                resonance_common::CcMode::Relative(_) => " rel",
            };
            format!("CC {cc}{rel} \u{b7} ch {}", channel + 1)
        }
        ControlSource::Note { channel, note } => {
            const NAMES: [&str; 12] =
                ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
            let octave = i32::from(note / 12) - 2;
            format!(
                "Note {}{octave} \u{b7} ch {}",
                NAMES[usize::from(note % 12)],
                channel + 1
            )
        }
    }
}

/// The short badge a bound control wears: `CC7`, `N36`.
pub fn source_badge(source: ControlSource) -> String {
    match source {
        ControlSource::Cc { cc, .. } => format!("CC{cc}"),
        ControlSource::Note { note, .. } => format!("N{note}"),
    }
}
