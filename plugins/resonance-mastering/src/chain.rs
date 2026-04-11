//! Mastering signal chain.
//!
//! Owns every DSP stage and the metering tap, orchestrating them in
//! processing order:
//!
//!   input → corrective EQ → glue compressor → saturator
//!         → tonal EQ → multiband → metering tap
//!
//! Later phases will add stereo imaging, the true-peak limiter, and
//! dither between the multiband and the meter.

use resonance_dsp::db_to_linear;

use crate::dsp::MeteringCore;
use crate::params::MasteringParams;
use crate::stages::dither::Dither;
use crate::stages::glue_compressor::GlueCompressor;
use crate::stages::imager::Imager;
use crate::stages::limiter::Limiter;
use crate::stages::linear_phase_eq::LinearPhaseEq;
use crate::stages::multiband::Multiband;
use crate::stages::saturator::Saturator;
use crate::viz::MasteringViz;

pub struct Chain {
    corrective_eq: LinearPhaseEq,
    glue_compressor: GlueCompressor,
    saturator: Saturator,
    tonal_eq: LinearPhaseEq,
    multiband: Multiband,
    imager: Imager,
    limiter: Limiter,
    dither: Dither,
    meters: MeteringCore,
    /// Smoothed input-trim gain (linear). Ramped toward the param's
    /// current value across each block so pushing the trim slider
    /// doesn't click.
    input_trim_lin: f32,
}

impl Chain {
    pub fn new(sample_rate: f32, max_buffer: usize, viz: &MasteringViz) -> Self {
        Self {
            corrective_eq: LinearPhaseEq::new(sample_rate),
            glue_compressor: GlueCompressor::new(sample_rate),
            saturator: Saturator::new(sample_rate),
            tonal_eq: LinearPhaseEq::new(sample_rate),
            multiband: Multiband::new(sample_rate, max_buffer),
            imager: Imager::new(sample_rate),
            limiter: Limiter::new(sample_rate),
            dither: Dither::new(),
            meters: MeteringCore::new(sample_rate, viz),
            input_trim_lin: 1.0,
        }
    }

    pub fn reset(&mut self) {
        self.corrective_eq.reset();
        self.glue_compressor.reset();
        self.saturator.reset();
        self.tonal_eq.reset();
        self.multiband.reset();
        self.imager.reset();
        self.limiter.reset();
        self.dither.reset();
        self.meters.reset();
        self.input_trim_lin = 1.0;
    }

    /// Total plugin latency in samples: sum of every latency-inducing
    /// stage. The compressor, saturator, and imager are zero-latency;
    /// the two linear-phase EQs and the multiband crossover each
    /// contribute one FIR convolver's worth of delay, and the limiter
    /// adds its lookahead.
    pub fn latency(&self) -> u32 {
        (self.corrective_eq.latency()
            + self.tonal_eq.latency()
            + Multiband::latency()
            + self.limiter.latency()) as u32
    }

    /// Run the chain on a stereo block. Audio is modified in place;
    /// the metering tap runs last so the meters reflect the final
    /// post-chain output.
    pub fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        params: &MasteringParams,
        viz: &MasteringViz,
    ) {
        // Input trim: linearly ramp from the last applied gain to the
        // current param value across the block so parameter changes
        // don't click. Runs on every block — identity trim is a
        // multiply by 1.0 and is cheap.
        let target_trim = db_to_linear(params.input_trim_db.value());
        let frames = left.len().min(right.len());
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

        let glue_cfg = params.glue_compressor.snapshot();
        self.glue_compressor.process_stereo(left, right, &glue_cfg);

        let sat_cfg = params.saturator.snapshot();
        self.saturator.process_stereo(left, right, &sat_cfg);

        let tonal_bands = params.tonal_eq.snapshot();
        self.tonal_eq.process_stereo(left, right, &tonal_bands);

        let mb_cfg = params.multiband.snapshot();
        self.multiband.process_stereo(left, right, &mb_cfg);

        let img_cfg = params.imager.snapshot();
        self.imager.process_stereo(left, right, &img_cfg);

        let lim_cfg = params.limiter.snapshot();
        self.limiter.process_stereo(left, right, &lim_cfg);

        let dither_cfg = params.dither.snapshot();
        self.dither.process_stereo(left, right, &dither_cfg);

        self.meters.feed(left, right, viz);

        // Publish the stage GR meters for the UI header.
        viz.store_gr(
            self.glue_compressor.meter_gr_db(),
            self.limiter.meter_gr_db(),
        );

        // Feed the post-chain audio into the assistant's capture ring.
        // Runs unconditionally so the user can click Analyze at any
        // moment without having to arm capture first.
        viz.assistant.feed(left, right);
    }
}
