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

/// Describes one audio output port exposed by a plugin. Returned from
/// `ResonancePlugin::output_layout()` at activation time; used by the CLAP
/// bridge to declare audio ports to the host and by the host mixer to size
/// its per-port scratch buffers.
#[derive(Debug, Clone)]
pub struct OutputPortSpec {
    /// Human-readable name shown to the host (e.g. "Out", "Kick", "Snare").
    pub name: String,
    /// Number of audio channels for this port. Only 1 (mono) and 2 (stereo)
    /// are supported right now; everything else is rejected at activation.
    pub channel_count: u32,
}

/// Mutable stereo buffer pair for one output port, passed to
/// `ResonancePlugin::process()` in a slice — one entry per declared port in
/// `output_layout()` order. Plugins write their output directly into
/// `left` and `right` (already zeroed when the process call begins).
pub struct OutputBuffer<'a> {
    pub left: &'a mut [f32],
    pub right: &'a mut [f32],
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

/// Transport/tempo snapshot delivered once per process block.
///
/// `None` when the host (or offline renderer) doesn't supply transport.
#[derive(Debug, Clone, Copy)]
pub struct TempoInfo {
    pub bpm: f32,
    pub time_sig_num: u16,
    pub time_sig_den: u16,
    pub playing: bool,
    pub song_pos_beats: f64,
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
    /// Whether this plugin accepts MIDI note input.
    const MIDI_INPUT: bool = false;

    /// Describe the plugin's audio output layout. Called once at activation
    /// and cached for the plugin's lifetime — **do not** change the port
    /// count across activations, the host caches it and sizes buffers
    /// accordingly. Default: a single stereo output named "Out".
    ///
    /// Port 0 is always the "main" output; plugins that declare multiple
    /// ports conventionally put their primary / mix-down output at index 0.
    fn output_layout(&self) -> Vec<OutputPortSpec> {
        vec![OutputPortSpec {
            name: "Out".to_string(),
            channel_count: 2,
        }]
    }

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
    /// `outputs` is a slice of stereo buffer pairs, one per declared output
    /// port in `output_layout()` order. Each buffer has `frames` samples
    /// and is already zeroed when the plugin is called (instrument path)
    /// or pre-filled with the incoming audio (effect path on port 0).
    ///
    /// Single-output plugins simply write into `outputs[0].left` /
    /// `outputs[0].right`. Multi-output plugins (e.g. resonance-drums with
    /// its 7 group/overhead ports) fan out to the full slice.
    ///
    /// `events` provides sample-accurate note events.
    fn process(
        &mut self,
        outputs: &mut [OutputBuffer<'_>],
        frames: usize,
        events: &mut EventIterator<'_>,
        tempo: Option<TempoInfo>,
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
