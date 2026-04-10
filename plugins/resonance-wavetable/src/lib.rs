/// Resonance Wavetable - A wavetable synthesizer instrument CLAP plugin.

use std::sync::Arc;

use resonance_plugin::*;

#[cfg(feature = "editor")]
mod editor;
mod effects;
mod engine;
mod envelope;
mod filter;
mod lfo;
mod modulation;
mod oscillator;
mod params;
mod viz;
mod voice;
mod wavetable;

use engine::SynthEngine;
use params::{WavetableParams, PARAM_COUNT};
use viz::WavetableVizState;

pub struct ResonanceWavetable {
    /// Parameters — shared with the editor thread via Arc so the UI can read
    /// and write from a separate thread. All `FloatParam` / `IntParam` /
    /// `BoolParam` fields use atomic storage internally, so `&WavetableParams`
    /// is safe to use concurrently from audio + UI.
    params: Arc<WavetableParams>,
    engine: SynthEngine,
    /// Shared audio-thread → UI-thread visualisation state. Lives as long as
    /// the plugin instance. Cloned into the editor factory when the host
    /// opens the GUI.
    viz: Arc<WavetableVizState>,
}

impl ResonancePlugin for ResonanceWavetable {
    const CLAP_ID: &'static str = "com.resonance.wavetable";
    const NAME: &'static str = "Resonance Wavetable";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "A wavetable synthesizer instrument";
    const FEATURES: &'static [&'static str] =
        &["instrument", "synthesizer", "stereo"];

    const INPUT_CHANNELS: Option<u32> = None;
    const OUTPUT_CHANNELS: u32 = 2;
    const MIDI_INPUT: bool = true;

    fn new() -> Self {
        Self {
            params: Arc::new(WavetableParams::new()),
            engine: SynthEngine::new(),
            viz: Arc::new(WavetableVizState::new()),
        }
    }

    fn param_count(&self) -> usize {
        PARAM_COUNT
    }

    fn param(&self, index: usize) -> &dyn Param {
        self.params.param_at(index)
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.engine.initialize(sample_rate);
        true
    }

    fn reset(&mut self) {
        self.engine.reset();
    }

    fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        frames: usize,
        events: &mut EventIterator<'_>,
    ) {
        resonance_common::flush_denormals();

        // Sample-accurate MIDI processing
        let mut next_event = events.next_event();

        for sample_id in 0..frames {
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }

                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => {
                        self.engine.note_on(note, velocity, &self.params);
                    }
                    NoteEvent::NoteOff { note, .. } => {
                        self.engine.note_off(note);
                    }
                    NoteEvent::Choke { note, .. } => {
                        self.engine.note_off(note);
                    }
                }

                next_event = events.next_event();
            }

            let (frame_l, frame_r) = self.engine.render_frame(&self.params);
            left[sample_id] = frame_l;
            right[sample_id] = frame_r;
        }

        // Publish audio-thread state to the shared viz atomics once per
        // block. The editor thread reads from these at ~60 Hz.
        self.engine.publish_viz(&self.params, &self.viz);
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::WavetableEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
        )))
    }
}

resonance_plugin::export_clap!(ResonanceWavetable);
