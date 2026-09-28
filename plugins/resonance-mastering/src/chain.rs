//! Mastering signal chain.
//!
//! Owns every DSP stage and the metering tap, orchestrating them in
//! processing order:
//!
//!   input trim → corrective EQ → de-harsh → glue compressor
//!         → saturator → tonal EQ → multiband → imager → clipper
//!         → limiter → dither → (de-harsh tail delay) → metering tap
//!
//! The imager's per-band width is applied on the multiband's crossover
//! bands, just before they are summed (see `stages::multiband`). Only
//! the two linear-phase EQs, the de-harsh STFT frame, the multiband
//! crossover and the limiter lookahead have latency, and it does not
//! depend on which stages are on or what they are set to. The de-harsh
//! delay sits either at its stage or at the chain's end, whichever keeps
//! a project that never engages it bit-identical to the chain without it
//! (see `stages::deharsh`).

use resonance_dsp::{db_to_linear, DelayLine};

use crate::dsp::MeteringCore;
use crate::params::MasteringParams;
use crate::stages::clipper::Clipper;
use crate::stages::deharsh::DeharshStage;
use crate::stages::dither::Dither;
use crate::stages::glue_compressor::GlueCompressor;
use crate::stages::imager::Imager;
use crate::stages::limiter::Limiter;
use crate::stages::linear_phase_eq::{DesignWorker, FirGeometry, LinearPhaseEq};
use crate::stages::multiband::Multiband;
use crate::stages::saturator::Saturator;
use crate::viz::MasteringViz;

pub struct Chain {
    corrective_eq: LinearPhaseEq,
    deharsh: DeharshStage,
    glue_compressor: GlueCompressor,
    saturator: Saturator,
    tonal_eq: LinearPhaseEq,
    multiband: Multiband,
    imager: Imager,
    clipper: Clipper,
    limiter: Limiter,
    dither: Dither,
    meters: MeteringCore,
    /// Latency-matched dry delay for whole-plugin bypass, mirroring the
    /// per-stage bypasses (multiband, limiter). Fed with the raw input
    /// on every block — bypassed or not — so toggling bypass switches
    /// to an already-warm delayed signal instead of zeros.
    bypass_delay_l: DelayLine,
    bypass_delay_r: DelayLine,
    /// Per-block scratch for the delayed dry signal (sized at
    /// `max_buffer`; longer host blocks are processed in chunks).
    dry_l: Vec<f32>,
    dry_r: Vec<f32>,
    /// Processed-path weight of the output, 1 = chain, 0 = delayed dry.
    /// Ramps over [`BYPASS_XFADE_SECONDS`] on bypass toggles.
    wet_mix: f32,
    /// Per-sample `wet_mix` step.
    wet_mix_step: f32,
    /// False until the first block: a bypass set before any audio
    /// engages instantly (there is nothing to click against).
    wet_mix_primed: bool,
    /// Smoothed input-trim gain (linear). Ramped toward the param's
    /// current value across each block so pushing the trim slider
    /// doesn't click.
    input_trim_lin: f32,
}

/// FIR convolvers in the chain: two EQs and three crossovers, stereo.
const CONVOLVER_COUNT: usize = 10;

/// Whole-plugin bypass crossfade length (DSP-05).
pub const BYPASS_XFADE_SECONDS: f32 = 0.010;

impl Chain {
    pub fn new(sample_rate: f32, max_buffer: usize, viz: &MasteringViz) -> Self {
        // One background FIR designer for all five linear-phase filters
        // (FU-M2a/FU-M2b); dropped with the chain, off the audio thread.
        let worker = DesignWorker::spawn();
        let mut corrective_eq = LinearPhaseEq::with_worker(sample_rate, Some(&worker));
        let mut tonal_eq = LinearPhaseEq::with_worker(sample_rate, Some(&worker));
        let mut multiband = Multiband::with_worker(sample_rate, max_buffer, Some(&worker));

        // Spread the ten convolvers' FFT iterations evenly over the hop
        // so no host callback runs more than one of them (DSP-16) — they
        // used to all fire in the same one. Latency is unaffected.
        let hop = FirGeometry::for_sample_rate(sample_rate).hop;
        let phase = |slot: usize| slot * hop / CONVOLVER_COUNT;
        corrective_eq.set_phase_offsets([phase(0), phase(5)]);
        tonal_eq.set_phase_offsets([phase(1), phase(6)]);
        multiband.set_phase_offsets([
            [phase(2), phase(7)],
            [phase(3), phase(8)],
            [phase(4), phase(9)],
        ]);
        // The EQs' mid/side cross pairs run only while a band is mid or
        // side; they take the half-slots in between, so even then no
        // callback runs two FFT iterations. (The ten above are left
        // exactly where they were: moving one changes its rounding.)
        let half_phase = |slot: usize| (2 * slot + 1) * hop / (2 * CONVOLVER_COUNT);
        corrective_eq.set_cross_phase_offsets([half_phase(0), half_phase(5)]);
        tonal_eq.set_cross_phase_offsets([half_phase(1), half_phase(6)]);
        let limiter = Limiter::new(sample_rate);
        let downstream_latency = tonal_eq.latency() + multiband.latency() + limiter.latency();
        let deharsh = DeharshStage::new(sample_rate, max_buffer, downstream_latency);
        let max_latency = corrective_eq.latency() + deharsh.latency() + downstream_latency;
        Self {
            corrective_eq,
            deharsh,
            glue_compressor: GlueCompressor::new(sample_rate),
            saturator: Saturator::new(sample_rate),
            tonal_eq,
            multiband,
            imager: Imager::new(sample_rate),
            clipper: Clipper::new(sample_rate),
            limiter,
            dither: Dither::new(),
            meters: MeteringCore::new(sample_rate, viz),
            bypass_delay_l: DelayLine::new(max_latency + 1),
            bypass_delay_r: DelayLine::new(max_latency + 1),
            dry_l: vec![0.0; max_buffer.max(1)],
            dry_r: vec![0.0; max_buffer.max(1)],
            wet_mix: 1.0,
            wet_mix_step: 1.0 / (BYPASS_XFADE_SECONDS * sample_rate).max(1.0),
            wet_mix_primed: false,
            input_trim_lin: 1.0,
        }
    }

    pub fn reset(&mut self) {
        self.corrective_eq.reset();
        self.deharsh.reset();
        self.glue_compressor.reset();
        self.saturator.reset();
        self.tonal_eq.reset();
        self.multiband.reset();
        self.imager.reset();
        self.clipper.reset();
        self.limiter.reset();
        self.dither.reset();
        self.meters.reset();
        self.bypass_delay_l.clear();
        self.bypass_delay_r.clear();
        self.wet_mix_primed = false;
        self.input_trim_lin = 1.0;
    }

    /// Total plugin latency in samples: sum of every latency-inducing
    /// stage. The compressor, saturator, imager and clipper are
    /// zero-latency (the clipper's oversampling is IIR);
    /// the two linear-phase EQs and the multiband crossover each
    /// contribute one FIR convolver's worth of delay, the de-harsh stage
    /// one STFT frame (on or off), and the limiter adds its lookahead.
    pub fn latency(&self) -> u32 {
        (self.corrective_eq.latency()
            + self.deharsh.latency()
            + self.tonal_eq.latency()
            + self.multiband.latency()
            + self.limiter.latency()) as u32
    }

    /// Samples until each of the ten FIR convolvers runs its next FFT
    /// iteration (corrective EQ L/R, tonal EQ L/R, crossovers 1–3 L/R).
    /// Diagnostics for the hop-phase stagger (DSP-16).
    pub fn convolver_iteration_countdowns(&self) -> [usize; CONVOLVER_COUNT] {
        let [c, t] = [
            self.corrective_eq.iteration_countdowns(),
            self.tonal_eq.iteration_countdowns(),
        ];
        let [x1, x2, x3] = self.multiband.iteration_countdowns();
        [c[0], c[1], t[0], t[1], x1[0], x1[1], x2[0], x2[1], x3[0], x3[1]]
    }

    /// Run the chain on a stereo block, honouring the whole-plugin
    /// `bypass` param. Audio is modified in place; the metering tap runs
    /// last so the meters reflect what the listener hears.
    ///
    /// Bypass swaps the output to the raw input delayed by exactly
    /// [`Chain::latency`] samples, so host delay compensation and A/B
    /// stay aligned, like the per-stage bypasses. The stages keep
    /// running on the input while bypassed (their output is discarded):
    /// frozen, they would hold the ~0.4 s of audio in flight when bypass
    /// engaged and replay it on un-bypass (DSP-05). Both directions
    /// crossfade over [`BYPASS_XFADE_SECONDS`]; at rest each path is
    /// passed bit-exactly.
    pub fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        params: &MasteringParams,
        viz: &MasteringViz,
    ) {
        let frames = left.len().min(right.len());
        let bypassed = params.bypass.value();
        let target = if bypassed { 0.0 } else { 1.0 };
        if !self.wet_mix_primed {
            self.wet_mix = target;
            self.wet_mix_primed = true;
        }
        let cap = self.dry_l.len();
        let mut start = 0;
        while start < frames {
            let end = (start + cap).min(frames);
            self.process_chunk(&mut left[start..end], &mut right[start..end], params, target);
            start = end;
        }
        let (left, right) = (&mut left[..frames], &mut right[..frames]);

        self.meters.feed(left, right, viz);
        if bypassed {
            return;
        }

        // Publish the stage GR meters for the UI header, and the four
        // per-band multiband GR values for the Multiband tab. All of
        // these are already computed by the stages themselves.
        viz.store_gr(
            self.glue_compressor.meter_gr_db(),
            self.limiter.meter_gr_db(),
        );
        viz.store_band_gr(self.multiband.band_gr_db());

        // Feed the post-chain audio into the assistant's capture ring.
        // Runs unconditionally so the user can click Analyze at any
        // moment without having to arm capture first.
        viz.assistant.feed(left, right);
    }

    /// One chunk of at most `dry_l.len()` frames: fill the delayed dry
    /// path, run every stage in place, then blend toward `target`.
    fn process_chunk(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        params: &MasteringParams,
        target: f32,
    ) {
        let frames = left.len().min(right.len());
        let delay = self.latency() as usize;
        for i in 0..frames {
            self.bypass_delay_l.push(left[i]);
            self.bypass_delay_r.push(right[i]);
            self.dry_l[i] = self.bypass_delay_l.tap(delay);
            self.dry_r[i] = self.bypass_delay_r.tap(delay);
        }

        self.run_stages(left, right, params);

        if self.wet_mix == target {
            if target == 0.0 {
                left.copy_from_slice(&self.dry_l[..frames]);
                right.copy_from_slice(&self.dry_r[..frames]);
            }
            return;
        }
        for i in 0..frames {
            self.wet_mix = if target > self.wet_mix {
                (self.wet_mix + self.wet_mix_step).min(target)
            } else {
                (self.wet_mix - self.wet_mix_step).max(target)
            };
            let (dl, dr) = (self.dry_l[i], self.dry_r[i]);
            left[i] = dl + (left[i] - dl) * self.wet_mix;
            right[i] = dr + (right[i] - dr) * self.wet_mix;
        }
    }

    /// Input trim and every stage, in processing order, in place.
    fn run_stages(&mut self, left: &mut [f32], right: &mut [f32], params: &MasteringParams) {
        let frames = left.len().min(right.len());

        // Input trim: linearly ramp from the last applied gain to the
        // current param value across the block so parameter changes
        // don't click. Runs on every block — identity trim is a
        // multiply by 1.0 and is cheap.
        let target_trim = db_to_linear(params.input_trim_db.value());
        if frames > 0 {
            let step = (target_trim - self.input_trim_lin) / frames as f32;
            let mut g = self.input_trim_lin;
            for i in 0..frames {
                g += step;
                left[i] *= g;
                right[i] *= g;
            }
            self.input_trim_lin = target_trim;
        }

        let corrective_bands = params.corrective_eq.snapshot();
        self.corrective_eq
            .process_stereo(left, right, &corrective_bands);

        let dh_cfg = params.deharsh.snapshot();
        self.deharsh.process_stage(left, right, &dh_cfg);

        let glue_cfg = params.glue_compressor.snapshot();
        self.glue_compressor.process_stereo(left, right, &glue_cfg);

        let sat_cfg = params.saturator.snapshot();
        self.saturator.process_stereo(left, right, &sat_cfg);

        let tonal_bands = params.tonal_eq.snapshot();
        self.tonal_eq.process_stereo(left, right, &tonal_bands);

        // The imager's per-band width rides on the multiband's crossover
        // (see `stages::multiband`); unity while the imager is off.
        let mb_cfg = params.multiband.snapshot();
        let band_width = params.imager.band_width_targets();
        self.multiband
            .process_stereo_with_width(left, right, &mb_cfg, &band_width);

        let img_cfg = params.imager.snapshot();
        self.imager.process_stereo(left, right, &img_cfg);

        let clip_cfg = params.clipper.snapshot();
        self.clipper.process_stereo(left, right, &clip_cfg);

        let lim_cfg = params.limiter.snapshot();
        self.limiter.process_stereo(left, right, &lim_cfg);

        let dither_cfg = params.dither.snapshot();
        self.dither.process_stereo(left, right, &dither_cfg);

        // Last: the de-harsh delay, when it is spent here rather than at
        // the stage (see `stages::deharsh`).
        self.deharsh.process_tail(left, right);
    }

    /// Deepest current de-harsh cut, dB.
    pub fn deharsh_max_cut_db(&self) -> f32 {
        self.deharsh.max_cut_db()
    }
}
