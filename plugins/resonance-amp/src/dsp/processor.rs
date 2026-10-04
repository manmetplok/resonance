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
    /// A model [`SwapFader::try_begin_swap`] refused because every
    /// parking slot it could fall back on was taken (FU-D1a1, DSP2-15):
    /// kept here — never dropped on the audio thread — and retried by
    /// [`Self::retry_pending_swap`] on a later block. `lib.rs` must not
    /// collect another model from the mailbox while this is occupied
    /// (see [`Self::has_pending_swap`]); the mailbox's single slot holds
    /// it safely in the meantime, and any further supersession there is
    /// dropped on the loader thread, not this one.
    pending_swap: Option<Box<dyn NamInference>>,
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
            pending_swap: None,
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
    ///
    /// Uses [`SwapFader::try_begin_swap`] (FU-D1a1): a NAM model's weight
    /// buffers are heap-heavy enough that `begin_swap`'s drop-in-place
    /// fallback would mean a large free inside the sample loop, so when
    /// every parking slot is taken the model is kept and retried by
    /// [`Self::retry_pending_swap`] instead of being dropped here. Call
    /// only when [`Self::has_pending_swap`] is false — see its doc.
    pub fn install_pending_model(&mut self, model: Box<dyn NamInference>) {
        if let Err(refused) = self.models.try_begin_swap(model) {
            self.pending_swap = Some(refused);
        }
    }

    /// Whether a model is stuck waiting for parking space to free up.
    /// While true, the caller (`lib.rs`) must not collect another model
    /// from the mailbox: there is nowhere RT-safe to put a second refused
    /// payload, and the mailbox's single slot already holds the next one
    /// safely (superseding it there drops on the loader thread, not this
    /// one).
    pub fn has_pending_swap(&self) -> bool {
        self.pending_swap.is_some()
    }

    /// Retry a model [`Self::install_pending_model`] had to defer.
    /// RT-safe to call every block regardless of whether anything is
    /// pending (FU-D1a1).
    pub fn retry_pending_swap(&mut self) {
        if let Some(model) = self.pending_swap.take() {
            if let Err(refused) = self.models.try_begin_swap(model) {
                self.pending_swap = Some(refused);
            }
        }
    }

    /// Test-only: replace the janitor-backed retire sink with the
    /// caller's own channel, so a test can fill every parking slot
    /// deterministically (e.g. a channel with no reader) instead of
    /// racing the real janitor thread `new()` spawns. The old sink's
    /// sender is dropped, which ends that thread (FU-D1a1).
    #[cfg(feature = "test-internals")]
    pub fn set_retire_sink_for_test(
        &mut self,
        sink: std::sync::mpsc::SyncSender<Box<dyn NamInference>>,
    ) {
        self.models.set_retire_sink(sink);
    }

    /// Whether a model is installed (main thread, between activations).
    pub fn has_model(&self) -> bool {
        self.models.active().is_some()
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

        let dry_active = self.models.active().is_none_or(|m| m.is_identity());
        if dry_active && self.models.is_settled() {
            // No model loaded (or an identity one standing in for a
            // missing model): only `input_gain * output_gain` is
            // applied, stereo and unfiltered. A fade into or out of an
            // identity model runs the ticking loop below, which takes
            // the same dry path under the fade gain.
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
                Some(model) if !model.is_identity() => {
                    // The NAM model is mono-by-design: a single
                    // tube/amp captured at one mic position. Sum
                    // L+R into mono before driving it so a stereo
                    // input contributes both channels; previously
                    // we dropped R entirely (and read it only for
                    // peak metering), making the plugin act as
                    // an L-only effect for stereo signals.
                    // A centred source is expected; see
                    // `process_settled` on a one-sided input (DSP2-14).
                    let input = 0.5 * (dry_l + dry_r) * input_gain;
                    let raw = model.process_sample(input) * output_gain * fade_gain;
                    (self.dc_l.process(raw), self.dc_r.process(raw))
                }
                _ => {
                    // No model, or an identity one: the stereo dry path.
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
            //
            // The sum expects a *centred* source (DSP2-14): `0.5·(l+r)`
            // is unity for a signal on both sides, and that is what a
            // DI reaches the amp as in Resonance — a mono track (the
            // default for audio tracks) captures its one input channel
            // and duplicates it to L/R, and a mono clip plays on both
            // sides. A signal on one side only (a stereo track with the
            // guitar on just one of its two inputs) arrives 6 dB low,
            // which a NAM capture hears as less drive, not just less
            // level. That case is a track set-up, not a plugin mode:
            // record the DI on a mono track, or add +6 dB Input Gain.
            // A louder-channel or per-sample max detector would distort
            // the waveform the model is driven by, and an input-mode
            // parameter is not worth a CLAP param for a mis-set track.
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
