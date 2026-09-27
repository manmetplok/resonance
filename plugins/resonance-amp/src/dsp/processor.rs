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
    /// Mono model input / output for the block path, one slot per frame
    /// (`MAX_BLOCK_FRAMES` long; larger host blocks run in chunks).
    mono_in: Vec<f32>,
    mono_out: Vec<f32>,
}

/// Frames the block scratch is pre-sized for. Hosts in this project run
/// 64-1024 frame blocks; anything larger is processed in chunks of this.
const MAX_BLOCK_FRAMES: usize = 4096;

impl Default for AmpProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl AmpProcessor {
    pub fn new() -> Self {
        // A displaced model is a large heap free (WaveNet/LSTM weight
        // buffers, potentially MBs); route it to a janitor thread so
        // `begin_swap`/`next` never run that free inside
        // `process_block`'s sample loop.
        let mut models = SwapFader::new(SWAP_FADE_SAMPLES);
        models.set_retire_sink(SwapFader::spawn_retire_janitor("resonance-amp-janitor"));
        Self {
            models,
            dc_l: DcBlocker::default(),
            dc_r: DcBlocker::default(),
            input_gain_smoother: Smoother::new(SmoothingStyle::Logarithmic(50.0)),
            output_gain_smoother: Smoother::new(SmoothingStyle::Logarithmic(50.0)),
            mono_in: vec![0.0; MAX_BLOCK_FRAMES],
            mono_out: vec![0.0; MAX_BLOCK_FRAMES],
        }
    }

    /// Configure smoothers and DC blockers for a new sample rate, and
    /// seed the smoothers with their current parameter values.
    pub fn initialize(&mut self, sample_rate: f32, input_gain: f32, output_gain: f32) {
        self.input_gain_smoother.set_sample_rate(sample_rate);
        self.output_gain_smoother.set_sample_rate(sample_rate);
        self.input_gain_smoother.reset(input_gain);
        self.output_gain_smoother.reset(output_gain);
        // Corner fixed in Hz so a 7-string's low B (31 Hz) keeps its
        // weight at every project rate (DSP-04).
        self.dc_l
            .set_cutoff(DcBlocker::DEFAULT_CUTOFF_HZ, sample_rate);
        self.dc_r
            .set_cutoff(DcBlocker::DEFAULT_CUTOFF_HZ, sample_rate);
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

        if self.models.is_settled() {
            // Steady state (every block but the ~1024 samples of a model
            // swap): no fade to tick, so the model runs a whole block at
            // once — the WaveNet's block forward pass is several times
            // cheaper per sample than sample-serial calls.
            let mut start = 0;
            while start < frames {
                let len = (frames - start).min(MAX_BLOCK_FRAMES);
                self.process_settled(
                    &mut left[start..start + len],
                    &mut right[start..start + len],
                    &mut peaks,
                );
                start += len;
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

    /// Block path for a settled fader with an active model: mono sum and
    /// input gain per frame, one `process_block` through the model, then
    /// output gain and DC blocking per frame. Per-sample math identical to
    /// the ticking loop with a fade gain of 1.0.
    fn process_settled(&mut self, left: &mut [f32], right: &mut [f32], peaks: &mut BlockPeaks) {
        let n = left.len();
        let Some(model) = self.models.active_mut() else {
            return;
        };
        let mono_in = &mut self.mono_in[..n];
        for ((m, &l), &r) in mono_in.iter_mut().zip(left.iter()).zip(right.iter()) {
            peaks.in_l = peaks.in_l.max(l.abs());
            peaks.in_r = peaks.in_r.max(r.abs());
            // The NAM model is mono-by-design (one amp at one mic
            // position): sum L+R so a stereo input contributes both.
            *m = 0.5 * (l + r) * self.input_gain_smoother.next();
        }
        let mono_out = &mut self.mono_out[..n];
        model.process_block(mono_in, mono_out);
        for ((l, r), &raw) in left.iter_mut().zip(right.iter_mut()).zip(mono_out.iter()) {
            let raw = raw * self.output_gain_smoother.next();
            let out_l = self.dc_l.process(raw);
            let out_r = self.dc_r.process(raw);
            *l = out_l;
            *r = out_r;
            peaks.out_l = peaks.out_l.max(out_l.abs());
            peaks.out_r = peaks.out_r.max(out_r.abs());
        }
    }
}
