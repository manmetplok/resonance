/// Resonance Drums - A drum sampler instrument CLAP plugin.

use nih_plug::prelude::*;
use std::sync::Arc;

mod drum_map;
mod kit;
mod params;
mod sampler;
mod voice;

use params::DrumParams;
use sampler::DrumSampler;

pub struct ResonanceDrums {
    params: Arc<DrumParams>,
    sampler: DrumSampler,
}

impl Default for ResonanceDrums {
    fn default() -> Self {
        Self {
            params: Arc::new(DrumParams::default()),
            sampler: DrumSampler::new(),
        }
    }
}

impl Plugin for ResonanceDrums {
    const NAME: &'static str = "Resonance Drums";
    const VENDOR: &'static str = "Resonance";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sampler.load_defaults(buffer_config.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.sampler.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
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
        let mut next_event = context.next_event();

        for (sample_id, channel_samples) in buffer.iter_samples().enumerate() {
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
                    _ => {}
                }

                next_event = context.next_event();
            }

            // Render one stereo frame from the sampler
            let mut left = 0.0f32;
            let mut right = 0.0f32;
            self.sampler
                .render_frame(&mut left, &mut right, &pad_volumes, &pad_pans);

            // Write to output with master volume
            let mut samples = channel_samples.into_iter();
            if let Some(out_l) = samples.next() {
                *out_l = left * master_vol;
            }
            if let Some(out_r) = samples.next() {
                *out_r = right * master_vol;
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for ResonanceDrums {
    const CLAP_ID: &'static str = "com.resonance.drums";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A drum sampler instrument");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Sampler,
        ClapFeature::Drum,
        ClapFeature::Stereo,
    ];
}

nih_export_clap!(ResonanceDrums);
