/// Resonance Reverb - An algorithmic reverb using diffusion networks and FDN.

use nih_plug::prelude::*;
use std::sync::Arc;

pub mod dsp;
pub mod params;

use dsp::ReverbDsp;
use params::ReverbParams;

pub struct ResonanceReverb {
    params: Arc<ReverbParams>,
    reverb: Option<ReverbDsp>,
}

impl Default for ResonanceReverb {
    fn default() -> Self {
        Self {
            params: Arc::new(ReverbParams::default()),
            reverb: None,
        }
    }
}

impl Plugin for ResonanceReverb {
    const NAME: &'static str = "Resonance Reverb";
    const VENDOR: &'static str = "Resonance";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
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
        self.reverb = Some(ReverbDsp::new(buffer_config.sample_rate));
        true
    }

    fn reset(&mut self) {
        if let Some(reverb) = &mut self.reverb {
            reverb.clear();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        resonance_common::flush_denormals();

        let Some(reverb) = &mut self.reverb else {
            return ProcessStatus::Normal;
        };

        let num_channels = buffer.channels();

        // Update reverb parameters (smoothed per-block for size/decay/damping/mod
        // since they recalculate internal state; mix/width/diffusion are per-sample)
        let size = self.params.size.smoothed.next();
        let decay = self.params.decay.smoothed.next();
        let damping = self.params.damping.smoothed.next();
        let predelay = self.params.predelay.smoothed.next();
        let mod_rate = self.params.mod_rate.smoothed.next();
        let mod_depth = self.params.mod_depth.smoothed.next();
        let freeze = self.params.freeze.value();

        reverb.set_size(size);
        reverb.set_decay(decay);
        reverb.set_damping(damping);
        reverb.set_predelay(predelay);
        reverb.set_mod_rate(mod_rate);
        reverb.set_mod_depth(mod_depth);
        reverb.set_freeze(freeze);

        for mut channel_samples in buffer.iter_samples() {
            let mix = self.params.mix.smoothed.next();
            let width = self.params.width.smoothed.next();
            let diffusion = self.params.diffusion.smoothed.next();

            let Some(sample_l) = channel_samples.get_mut(0) else {
                continue;
            };
            let dry_l = *sample_l;
            let dry_r = if num_channels >= 2 {
                *channel_samples.get_mut(1).unwrap()
            } else {
                dry_l
            };

            let (wet_l, wet_r) = reverb.process(dry_l, dry_r, diffusion, width);

            let dry_amount = 1.0 - mix;
            let out_l = dry_l * dry_amount + wet_l * mix;
            let out_r = dry_r * dry_amount + wet_r * mix;

            *channel_samples.get_mut(0).unwrap() = out_l;
            if num_channels >= 2 {
                *channel_samples.get_mut(1).unwrap() = out_r;
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for ResonanceReverb {
    const CLAP_ID: &'static str = "com.resonance.reverb";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Algorithmic reverb with diffusion network and FDN");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Reverb,
    ];
}

nih_export_clap!(ResonanceReverb);
