//! Audio-thread processor for the amp.
//!
//! Owns the NAM model slots, the DC blockers, and the gain smoothers — i.e.
//! everything that lives strictly on the audio thread. `lib.rs` keeps the
//! shared/mailbox state and is responsible for handing newly-loaded models
//! to the processor via [`AmpProcessor::install_pending_model`].

use resonance_dsp::{DcBlocker, SwapFader};
use resonance_plugin::{Smoother, SmoothingStyle};

use crate::nam::NamInference;

/// Crossfade length in samples (~23 ms at 44.1 kHz). Long enough to
/// mask any residual transient when a freshly-loaded model takes over
/// mid-audio, even after the loader thread has primed it.
pub(crate) const SWAP_FADE_SAMPLES: u32 = 1024;

/// In/out peak amplitudes captured across a block, in linear units.
#[derive(Default, Clone, Copy)]
pub struct BlockPeaks {
    pub in_l: f32,
    pub in_r: f32,
    pub out_l: f32,
    pub out_r: f32,
}

/// Audio-thread NAM model runner: fades between models, smooths gain
/// parameters, applies DC blocking, and reports input/output peaks.
pub struct AmpProcessor {
    models: SwapFader<Box<dyn NamInference>>,
    dc_l: DcBlocker,
    dc_r: DcBlocker,
    input_gain_smoother: Smoother,
    output_gain_smoother: Smoother,
}

impl AmpProcessor {
    pub fn new() -> Self {
        Self {
            models: SwapFader::new(SWAP_FADE_SAMPLES),
            dc_l: DcBlocker::default(),
            dc_r: DcBlocker::default(),
            input_gain_smoother: Smoother::new(SmoothingStyle::Logarithmic(50.0)),
            output_gain_smoother: Smoother::new(SmoothingStyle::Logarithmic(50.0)),
        }
    }

    /// Configure smoothers and DC blockers for a new sample rate, and
    /// seed the smoothers with their current parameter values.
    pub fn initialize(&mut self, sample_rate: f32, input_gain: f32, output_gain: f32) {
        self.input_gain_smoother.set_sample_rate(sample_rate);
        self.output_gain_smoother.set_sample_rate(sample_rate);
        self.input_gain_smoother.reset(input_gain);
        self.output_gain_smoother.reset(output_gain);
        self.dc_l.reset();
        self.dc_r.reset();
    }

    /// Reset DC blockers and the active model (used by the host's
    /// `reset()` hook).
    pub fn reset(&mut self) {
        if let Some(model) = self.models.active_mut() {
            model.reset();
        }
        self.dc_l.reset();
        self.dc_r.reset();
    }

    /// Install a model that has just landed in the mailbox. If a model
    /// is already active, kicks off a fade-out so the swap happens
    /// transparently mid-block.
    pub fn install_pending_model(&mut self, model: Box<dyn NamInference>) {
        self.models.begin_swap(model);
    }

    /// Install the very first model synchronously, with no crossfade and
    /// no fade-in. Used during plugin initialization before `process()`
    /// has had a chance to run.
    pub fn install_initial_model(&mut self, model: Box<dyn NamInference>) {
        self.models.install(model);
    }

    /// Set smoother targets for the upcoming block.
    pub fn set_gain_targets(&mut self, input_gain: f32, output_gain: f32) {
        self.input_gain_smoother.set_target(input_gain);
        self.output_gain_smoother.set_target(output_gain);
    }

    /// Run the per-sample NAM/bypass/crossfade loop over `frames` samples
    /// of `left` and `right`. Returns the linear input/output peaks
    /// observed across the block.
    pub fn process_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        frames: usize,
    ) -> BlockPeaks {
        let mut peaks = BlockPeaks::default();

        if self.models.active().is_none() && !self.models.is_fading_out() {
            // No model loaded: only `input_gain * output_gain` is
            // applied. The fader is idle here — a fade-in is always
            // paired with a newly installed model.
            for i in 0..frames {
                let dry_l = left[i];
                let dry_r = right[i];
                peaks.in_l = peaks.in_l.max(dry_l.abs());
                peaks.in_r = peaks.in_r.max(dry_r.abs());

                let input_gain = self.input_gain_smoother.next();
                let output_gain = self.output_gain_smoother.next();
                let gain = input_gain * output_gain;
                let out_l = dry_l * gain;
                let out_r = dry_r * gain;
                left[i] = out_l;
                right[i] = out_r;
                peaks.out_l = peaks.out_l.max(out_l.abs());
                peaks.out_r = peaks.out_r.max(out_r.abs());
            }
            return peaks;
        }

        for i in 0..frames {
            let dry_l = left[i];
            let dry_r = right[i];
            peaks.in_l = peaks.in_l.max(dry_l.abs());
            peaks.in_r = peaks.in_r.max(dry_r.abs());

            let input_gain = self.input_gain_smoother.next();
            let output_gain = self.output_gain_smoother.next();
            // Ticks the swap crossfade; replaces the model mid-block
            // when a pending one finishes fading out.
            let (fade_gain, model) = self.models.next();

            let (out_l, out_r) = match model {
                Some(model) => {
                    // The NAM model is mono-by-design: a single
                    // tube/amp captured at one mic position. Sum
                    // L+R into mono before driving it so a stereo
                    // input contributes both channels; previously
                    // we dropped R entirely (and read it only for
                    // peak metering), making the plugin act as
                    // an L-only effect for stereo signals.
                    let input = 0.5 * (dry_l + dry_r) * input_gain;
                    let raw = model.process_sample(input) * output_gain * fade_gain;
                    (self.dc_l.process(raw), self.dc_r.process(raw))
                }
                None => {
                    let gain = input_gain * output_gain * fade_gain;
                    (dry_l * gain, dry_r * gain)
                }
            };
            left[i] = out_l;
            right[i] = out_r;
            peaks.out_l = peaks.out_l.max(out_l.abs());
            peaks.out_r = peaks.out_r.max(out_r.abs());
        }

        peaks
    }
}
