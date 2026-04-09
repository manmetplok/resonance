/// Resonance Reverb - An algorithmic reverb using diffusion networks and FDN.

use resonance_plugin::*;

pub mod dsp;
pub mod params;

use dsp::ReverbDsp;
use params::ReverbParams;

pub struct ResonanceReverb {
    params: ReverbParams,
    reverb: Option<ReverbDsp>,
}

impl ResonancePlugin for ResonanceReverb {
    const CLAP_ID: &'static str = "com.resonance.reverb";
    const NAME: &'static str = "Resonance Reverb";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "Algorithmic reverb with diffusion network and FDN";
    const FEATURES: &'static [&'static str] = &["audio-effect", "stereo", "reverb"];

    const INPUT_CHANNELS: Option<u32> = Some(2);
    const OUTPUT_CHANNELS: u32 = 2;

    fn new() -> Self {
        Self {
            params: ReverbParams::default(),
            reverb: None,
        }
    }

    fn param_count(&self) -> usize { 10 }

    fn param(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.params.predelay,
            1 => &self.params.size,
            2 => &self.params.decay,
            3 => &self.params.damping,
            4 => &self.params.diffusion,
            5 => &self.params.mod_rate,
            6 => &self.params.mod_depth,
            7 => &self.params.width,
            8 => &self.params.mix,
            9 => &self.params.freeze,
            _ => unreachable!("invalid param index {index}"),
        }
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        // Set smoother sample rates
        self.params.predelay.smoother.set_sample_rate(sample_rate);
        self.params.size.smoother.set_sample_rate(sample_rate);
        self.params.decay.smoother.set_sample_rate(sample_rate);
        self.params.damping.smoother.set_sample_rate(sample_rate);
        self.params.diffusion.smoother.set_sample_rate(sample_rate);
        self.params.mod_rate.smoother.set_sample_rate(sample_rate);
        self.params.mod_depth.smoother.set_sample_rate(sample_rate);
        self.params.width.smoother.set_sample_rate(sample_rate);
        self.params.mix.smoother.set_sample_rate(sample_rate);

        // Initialize smoother targets to current values
        self.params.predelay.smoother.reset(self.params.predelay.value());
        self.params.size.smoother.reset(self.params.size.value());
        self.params.decay.smoother.reset(self.params.decay.value());
        self.params.damping.smoother.reset(self.params.damping.value());
        self.params.diffusion.smoother.reset(self.params.diffusion.value());
        self.params.mod_rate.smoother.reset(self.params.mod_rate.value());
        self.params.mod_depth.smoother.reset(self.params.mod_depth.value());
        self.params.width.smoother.reset(self.params.width.value());
        self.params.mix.smoother.reset(self.params.mix.value());

        self.reverb = Some(ReverbDsp::new(sample_rate));
        true
    }

    fn reset(&mut self) {
        if let Some(reverb) = &mut self.reverb {
            reverb.clear();
        }
    }

    fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        frames: usize,
        _events: &mut EventIterator<'_>,
    ) {
        resonance_common::flush_denormals();

        let Some(reverb) = &mut self.reverb else {
            return;
        };

        // Update reverb parameters (smoothed per-block for size/decay/damping/mod)
        // Set smoother targets from current param values
        self.params.size.smoother.set_target(self.params.size.value());
        self.params.decay.smoother.set_target(self.params.decay.value());
        self.params.damping.smoother.set_target(self.params.damping.value());
        self.params.predelay.smoother.set_target(self.params.predelay.value());
        self.params.mod_rate.smoother.set_target(self.params.mod_rate.value());
        self.params.mod_depth.smoother.set_target(self.params.mod_depth.value());
        self.params.mix.smoother.set_target(self.params.mix.value());
        self.params.width.smoother.set_target(self.params.width.value());
        self.params.diffusion.smoother.set_target(self.params.diffusion.value());

        let freeze = self.params.freeze.value();

        for i in 0..frames {
            let size = self.params.size.smoother.next();
            let decay = self.params.decay.smoother.next();
            let damping = self.params.damping.smoother.next();
            let predelay = self.params.predelay.smoother.next();
            let mod_rate = self.params.mod_rate.smoother.next();
            let mod_depth = self.params.mod_depth.smoother.next();
            let mix = self.params.mix.smoother.next();
            let width = self.params.width.smoother.next();
            let diffusion = self.params.diffusion.smoother.next();

            reverb.set_size(size);
            reverb.set_decay(decay);
            reverb.set_freeze(freeze);
            reverb.set_damping(damping);
            reverb.set_predelay(predelay);
            reverb.set_mod_rate(mod_rate);
            reverb.set_mod_depth(mod_depth);

            let dry_l = left[i];
            let dry_r = right[i];

            let (wet_l, wet_r) = reverb.process(dry_l, dry_r, diffusion, width);

            let dry_amount = 1.0 - mix;
            left[i] = dry_l * dry_amount + wet_l * mix;
            right[i] = dry_r * dry_amount + wet_r * mix;
        }
    }
}

resonance_plugin::export_clap!(ResonanceReverb);
