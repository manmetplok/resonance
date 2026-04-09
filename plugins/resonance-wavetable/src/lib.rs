/// Resonance Wavetable - A wavetable synthesizer instrument CLAP plugin.

use resonance_plugin::*;

mod effects;
mod engine;
mod envelope;
mod filter;
mod lfo;
mod modulation;
mod oscillator;
mod params;
mod voice;
mod wavetable;

use engine::SynthEngine;
use params::{WavetableParams, PARAM_COUNT};

pub struct ResonanceWavetable {
    params: WavetableParams,
    engine: SynthEngine,
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
            params: WavetableParams::new(),
            engine: SynthEngine::new(),
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
    }
}

#[cfg(not(feature = "ui"))]
resonance_plugin::export_clap!(ResonanceWavetable);
