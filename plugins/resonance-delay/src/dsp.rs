use resonance_dsp::{Biquad, DelayLine, OnePole};

use crate::params::DelaySmoothers;

/// Block-level parameters that do not change within a process call.
/// Per-sample smoothed parameters (`delay_samples`, `feedback`, `mix`)
/// come off the `DelaySmoothers` passed alongside.
pub struct BlockParams {
    pub character: i32,
    pub routing: i32,
    pub stereo_offset: f32,
    pub drive: f32,
    pub mod_rate: f32,
    pub mod_depth: f32,
    pub freeze: bool,
}

/// In/out peak amplitudes captured across a block, in linear units.
#[derive(Default)]
pub struct BlockPeaks {
    pub in_l: f32,
    pub in_r: f32,
    pub out_l: f32,
    pub out_r: f32,
}

pub struct DelayDsp {
    sample_rate: f32,
    max_delay_samples: f32,
    delay_l: DelayLine,
    delay_r: DelayLine,
    hp_l: Biquad,
    hp_r: Biquad,
    lp_l: OnePole,
    lp_r: OnePole,
    lfo_phase: f32,
}

impl DelayDsp {
    pub fn new(sample_rate: f32) -> Self {
        let max_samples = (sample_rate * 4.0) as usize + 256;
        let mut lp_l = OnePole::new();
        let mut lp_r = OnePole::new();
        lp_l.set_cutoff(8000.0, sample_rate);
        lp_r.set_cutoff(8000.0, sample_rate);

        Self {
            sample_rate,
            max_delay_samples: max_samples as f32,
            delay_l: DelayLine::new(max_samples),
            delay_r: DelayLine::new(max_samples),
            hp_l: Biquad::identity(),
            hp_r: Biquad::identity(),
            lp_l,
            lp_r,
            lfo_phase: 0.0,
        }
    }

    /// Set tone filter coefficients once per block to avoid expensive
    /// trig recomputation on every sample.
    pub fn set_tone_filters(&mut self, hi_cut: f32, lo_cut: f32, character: i32, delay_samples: f32) {
        let delay_sec = delay_samples / self.sample_rate;
        let effective_hi_cut = if character == 1 {
            hi_cut * (-delay_sec * 0.6).exp()
        } else {
            hi_cut
        };
        self.lp_l.set_cutoff(effective_hi_cut, self.sample_rate);
        self.lp_r.set_cutoff(effective_hi_cut, self.sample_rate);
        self.hp_l.set_high_pass(self.sample_rate, lo_cut, 0.707);
        self.hp_r.set_high_pass(self.sample_rate, lo_cut, 0.707);
    }

    pub fn clear(&mut self) {
        self.delay_l.clear();
        self.delay_r.clear();
        self.hp_l.reset();
        self.hp_r.reset();
        self.lp_l.clear();
        self.lp_r.clear();
        self.lfo_phase = 0.0;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        in_l: f32,
        in_r: f32,
        delay_samples: f32,
        feedback: f32,
        character: i32,
        routing: i32,
        stereo_offset: f32,
        drive: f32,
        mod_rate: f32,
        mod_depth: f32,
        freeze: bool,
    ) -> (f32, f32) {
        // Per-sample libm sine for the wow/flutter LFO. Deliberate
        // tradeoff: one `sin` per frame is noise next to the two
        // linear-interp delay taps below, and the LFO must stay
        // smooth at arbitrary (automatable) rates. Swap in a
        // polynomial approximation only if profiling ever shows this
        // hot — it never has.
        let lfo_val = (self.lfo_phase * std::f32::consts::TAU).sin();
        self.lfo_phase += mod_rate / self.sample_rate;
        self.lfo_phase -= self.lfo_phase.floor();

        let wow_range = 0.01 * self.sample_rate;
        let effective_mod = if character == 1 {
            mod_depth * 1.5
        } else {
            mod_depth
        };
        let wow = lfo_val * effective_mod * wow_range;

        let delay_l_samp = (delay_samples + wow).clamp(1.0, self.max_delay_samples - 4.0);
        let delay_r_samp = match routing {
            2 => {
                // Dual: apply stereo offset
                let offset = stereo_offset * delay_samples;
                (delay_samples + offset + wow).clamp(1.0, self.max_delay_samples - 4.0)
            }
            _ => delay_l_samp,
        };

        let wet_l = self.delay_l.tap_linear(delay_l_samp);
        let wet_r = self.delay_r.tap_linear(delay_r_samp);

        // Tone filtering: coefficients are set per-block via set_tone_filters().
        let filt_l = self.hp_l.process(self.lp_l.process(wet_l));
        let filt_r = self.hp_r.process(self.lp_r.process(wet_r));

        // Saturation on feedback path.
        let drive_amt = if character == 1 {
            1.0 + 3.0 * (drive + 0.1)
        } else {
            1.0 + 3.0 * drive
        };
        let sat_l = (filt_l * drive_amt).tanh();
        let sat_r = (filt_r * drive_amt).tanh();

        let fb = if freeze { 1.0 } else { feedback };
        let fb_l = sat_l * fb;
        let fb_r = sat_r * fb;

        match routing {
            1 => {
                // Ping-Pong: mono input → L, cross-feedback L↔R.
                let mono_in = if freeze { 0.0 } else { (in_l + in_r) * 0.5 };
                self.delay_l.push(mono_in + fb_r);
                self.delay_r.push(fb_l);
            }
            2 => {
                // Dual: mono input to both lines with stereo offset.
                let mono_in = if freeze { 0.0 } else { (in_l + in_r) * 0.5 };
                self.delay_l.push(mono_in + fb_l);
                self.delay_r.push(mono_in + fb_r);
            }
            _ => {
                // Stereo: L and R are independent.
                let inp_l = if freeze { 0.0 } else { in_l };
                let inp_r = if freeze { 0.0 } else { in_r };
                self.delay_l.push(inp_l + fb_l);
                self.delay_r.push(inp_r + fb_r);
            }
        }

        (wet_l, wet_r)
    }

    /// Render one process block. Reads per-sample-smoothed
    /// `delay_samples`, `feedback`, and `mix` off the supplied
    /// `smoothers` (so the caller only retargets them once per block),
    /// runs the inner delay-line loop, mixes wet/dry in place into the
    /// supplied buffers, and returns the in/out peak amplitudes for VU
    /// metering.
    pub fn process_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        frames: usize,
        smoothers: &mut DelaySmoothers,
        params: &BlockParams,
    ) -> BlockPeaks {
        let mut peaks = BlockPeaks::default();
        for i in 0..frames {
            let delay_samp = smoothers.delay_samples.next();
            let feedback = smoothers.feedback.next();
            let mix = smoothers.mix.next();

            let dry_l = left[i];
            let dry_r = right[i];
            peaks.in_l = peaks.in_l.max(dry_l.abs());
            peaks.in_r = peaks.in_r.max(dry_r.abs());

            let (wet_l, wet_r) = self.process(
                dry_l,
                dry_r,
                delay_samp,
                feedback,
                params.character,
                params.routing,
                params.stereo_offset,
                params.drive,
                params.mod_rate,
                params.mod_depth,
                params.freeze,
            );

            let dry_amount = 1.0 - mix;
            let out_l = dry_l * dry_amount + wet_l * mix;
            let out_r = dry_r * dry_amount + wet_r * mix;
            left[i] = out_l;
            right[i] = out_r;
            peaks.out_l = peaks.out_l.max(out_l.abs());
            peaks.out_r = peaks.out_r.max(out_r.abs());
        }
        peaks
    }
}
