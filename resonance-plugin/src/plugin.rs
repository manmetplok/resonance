/// The ResonancePlugin trait -- what plugin authors implement.

use crate::param::Param;

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

    /// Save plugin state to bytes. Default: JSON serialization of params.
    fn save_state(&self) -> Vec<u8> {
        crate::state::save_params(&self.params())
    }

    /// Load plugin state from bytes. Default: JSON deserialization of params.
    fn load_state(&mut self, data: &[u8]) -> bool {
        crate::state::load_params(&self.params(), data)
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
