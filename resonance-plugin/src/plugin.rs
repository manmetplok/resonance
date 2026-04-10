/// The ResonancePlugin trait -- what plugin authors implement.

use std::sync::Arc;

use crate::param::Param;

/// Saver for plugin state that lives outside the parameter list — file
/// paths, loaded resource handles, anything the plugin needs to persist
/// alongside its params.
///
/// The CLAP bridge harvests this handle once at plugin construction and
/// calls it from the main thread at project save / project load time.
/// Because the bridge may call `save`/`load` **while the plugin is in the
/// audio processor**, implementations must only touch thread-safe shared
/// state (Arcs, atomics, parking_lot mutexes) — never fields that the
/// plugin struct owns exclusively.
///
/// Keys returned from `save` are merged into the top level of the state
/// JSON object alongside `"params"`, so existing on-disk state formats
/// that use top-level keys remain readable.
pub trait ExtraStateSaver: Send + Sync {
    /// Return key-value pairs to merge into the top-level state JSON.
    fn save(&self) -> serde_json::Map<String, serde_json::Value>;

    /// Apply previously-saved state from the top-level JSON object.
    /// Implementations typically `state.get("my_key")` into their own
    /// shared storage.
    fn load(&self, state: &serde_json::Value);
}

/// A note event for sample-accurate MIDI processing.
#[derive(Debug, Clone, Copy)]
pub enum NoteEvent {
    NoteOn {
        note: u8,
        velocity: f32,
        timing: u32,
    },
    NoteOff {
        note: u8,
        timing: u32,
    },
    Choke {
        note: u8,
        timing: u32,
    },
}

impl NoteEvent {
    pub fn timing(&self) -> u32 {
        match self {
            NoteEvent::NoteOn { timing, .. } => *timing,
            NoteEvent::NoteOff { timing, .. } => *timing,
            NoteEvent::Choke { timing, .. } => *timing,
        }
    }
}

/// Iterator over note events within a process block.
/// Borrows from a pre-allocated buffer to avoid audio-thread allocations.
pub struct EventIterator<'a> {
    events: &'a [NoteEvent],
    pos: usize,
}

impl<'a> EventIterator<'a> {
    pub fn new(events: &'a [NoteEvent]) -> Self {
        Self { events, pos: 0 }
    }

    pub fn empty() -> Self {
        Self {
            events: &[],
            pos: 0,
        }
    }

    /// Peek at the next event without consuming it.
    pub fn peek(&self) -> Option<&NoteEvent> {
        self.events.get(self.pos)
    }

    /// Consume and return the next event.
    pub fn next_event(&mut self) -> Option<NoteEvent> {
        if self.pos < self.events.len() {
            let event = self.events[self.pos];
            self.pos += 1;
            Some(event)
        } else {
            None
        }
    }
}

/// The main trait that plugin authors implement.
///
/// The CLAP bridge wraps this trait to produce a valid CLAP plugin.
pub trait ResonancePlugin: Send + 'static {
    /// CLAP plugin identifier (e.g. "com.resonance.reverb").
    const CLAP_ID: &'static str;
    /// Human-readable plugin name.
    const NAME: &'static str;
    /// Plugin vendor name.
    const VENDOR: &'static str;
    /// Plugin version string.
    const VERSION: &'static str;
    /// Short description.
    const DESCRIPTION: &'static str;
    /// CLAP feature strings (e.g. "audio-effect", "stereo", "reverb").
    const FEATURES: &'static [&'static str];

    /// Number of input channels. None = instrument (no audio input).
    const INPUT_CHANNELS: Option<u32>;
    /// Number of output channels.
    const OUTPUT_CHANNELS: u32;
    /// Whether this plugin accepts MIDI note input.
    const MIDI_INPUT: bool = false;

    /// Create a new instance of the plugin.
    fn new() -> Self;

    /// Return the number of parameters.
    fn param_count(&self) -> usize;

    /// Return a reference to the parameter at the given index.
    fn param(&self, index: usize) -> &dyn Param;

    /// Return all parameters as a Vec (convenience, allocates).
    /// Default implementation builds from param_count/param.
    fn params(&self) -> Vec<&dyn Param> {
        (0..self.param_count()).map(|i| self.param(i)).collect()
    }

    /// Called once before processing begins. Return false on failure.
    fn initialize(&mut self, sample_rate: f32, max_buffer_size: u32) -> bool;

    /// Reset all internal state (e.g. delay lines, filters).
    fn reset(&mut self);

    /// Process a buffer of audio.
    ///
    /// `left` and `right` are mutable slices of `frames` length.
    /// For instruments (no input), they are zeroed before calling.
    /// `events` provides sample-accurate note events.
    fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        frames: usize,
        events: &mut EventIterator<'_>,
    );

    /// Save plugin state to bytes. Default: JSON serialization of params
    /// composed with whatever `extra_state_saver()` returns (so plugins that
    /// just need a couple of extra file-path fields can skip overriding
    /// this entirely and provide a saver instead).
    fn save_state(&self) -> Vec<u8> {
        let mut json = crate::state::params_to_json(&self.params());
        if let Some(saver) = self.extra_state_saver() {
            if let Some(obj) = json.as_object_mut() {
                for (k, v) in saver.save() {
                    obj.insert(k, v);
                }
            }
        }
        serde_json::to_vec(&json).unwrap_or_default()
    }

    /// Load plugin state from bytes. Default: JSON deserialization of
    /// params plus any `extra_state_saver()` contribution.
    fn load_state(&mut self, data: &[u8]) -> bool {
        let Ok(state) = serde_json::from_slice::<serde_json::Value>(data) else {
            return false;
        };
        let ok = crate::state::load_params_from_json(&self.params(), &state);
        if let Some(saver) = self.extra_state_saver() {
            saver.load(&state);
        }
        ok
    }

    /// Optional handle that persists state outside the param list (file
    /// paths, resource pointers, etc.). Harvested once at plugin creation
    /// by the CLAP bridge and cached for the plugin's lifetime, so the
    /// bridge can save/load extra state even while the plugin is in the
    /// audio processor. Default: `None`.
    fn extra_state_saver(&self) -> Option<Arc<dyn ExtraStateSaver>> {
        None
    }

    /// Report latency in samples. Default: 0.
    fn latency_samples(&self) -> u32 {
        0
    }

    /// Return an editor factory if this plugin has a GUI.
    ///
    /// Called once at plugin creation time (before the plugin is moved into
    /// the audio processor). Returning `Some(factory)` causes the clap_bridge
    /// to expose `CLAP_EXT_GUI`; the host can then open, resize, hide, show,
    /// and destroy the editor through the factory. Default: `None`.
    fn editor_factory(&self) -> Option<std::sync::Arc<dyn crate::gui::EditorFactory>> {
        None
    }
}
