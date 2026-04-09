/// Resonance Drums - A drum sampler instrument CLAP plugin.

use resonance_plugin::*;

mod drum_map;
mod kit;
mod params;
mod sampler;
mod voice;

#[cfg(feature = "ui")]
pub mod ui;

use params::DrumParams;
use sampler::DrumSampler;

pub struct ResonanceDrums {
    params: DrumParams,
    sampler: DrumSampler,
}

impl ResonancePlugin for ResonanceDrums {
    const CLAP_ID: &'static str = "com.resonance.drums";
    const NAME: &'static str = "Resonance Drums";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "A drum sampler instrument";
    const FEATURES: &'static [&'static str] = &["instrument", "sampler", "drum", "stereo"];

    const INPUT_CHANNELS: Option<u32> = None;
    const OUTPUT_CHANNELS: u32 = 2;
    const MIDI_INPUT: bool = true;

    fn new() -> Self {
        Self {
            params: DrumParams::default(),
            sampler: DrumSampler::new(),
        }
    }

    fn param_count(&self) -> usize {
        1 + drum_map::NUM_PADS * 3 // master_volume + (volume, pan, mute) per pad
    }

    fn param(&self, index: usize) -> &dyn Param {
        if index == 0 {
            return &self.params.master_volume;
        }
        let pad_idx = (index - 1) / 3;
        let field = (index - 1) % 3;
        let pad = &self.params.pads[pad_idx];
        match field {
            0 => &pad.volume,
            1 => &pad.pan,
            2 => &pad.mute,
            _ => unreachable!(),
        }
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.sampler.load_defaults(sample_rate);
        true
    }

    fn reset(&mut self) {
        self.sampler.reset();
    }

    fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        frames: usize,
        events: &mut EventIterator<'_>,
    ) {
        resonance_common::flush_denormals();

        // Read per-pad parameters
        let mut pad_volumes = [0.0f32; drum_map::NUM_PADS];
        let mut pad_pans = [0.0f32; drum_map::NUM_PADS];
        for (i, pad) in self.params.pads.iter().enumerate() {
            pad_volumes[i] = if pad.mute.value() {
                0.0
            } else {
                pad.volume.value()
            };
            pad_pans[i] = pad.pan.value();
        }
        let master_vol = self.params.master_volume.value();

        // Sample-accurate MIDI processing
        let mut next_event = events.next_event();

        for sample_id in 0..frames {
            // Process all MIDI events at this sample position
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }

                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => {
                        self.sampler.note_on(note, velocity);
                    }
                    NoteEvent::NoteOff { note, .. } => {
                        self.sampler.note_off(note);
                    }
                    NoteEvent::Choke { note, .. } => {
                        self.sampler.note_off(note);
                    }
                }

                next_event = events.next_event();
            }

            // Render one stereo frame from the sampler
            let mut frame_l = 0.0f32;
            let mut frame_r = 0.0f32;
            self.sampler
                .render_frame(&mut frame_l, &mut frame_r, &pad_volumes, &pad_pans);

            // Write to output with master volume
            left[sample_id] = frame_l * master_vol;
            right[sample_id] = frame_r * master_vol;
        }
    }
}

#[cfg(not(feature = "ui"))]
resonance_plugin::export_clap!(ResonanceDrums);
