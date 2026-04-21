/// Post-voice effects: distortion, chorus, stereo delay.
use resonance_dsp::{DelayLine, OnePole};

// ---------------------------------------------------------------------------
// Distortion (tanh soft-clip waveshaper)
// ---------------------------------------------------------------------------

pub struct Distortion;

impl Distortion {
    /// Process a stereo pair through distortion.
    #[inline]
    pub fn process(left: f32, right: f32, drive: f32, mix: f32) -> (f32, f32) {
        let dl = (left * drive).tanh();
        let dr = (right * drive).tanh();
        (
            left * (1.0 - mix) + dl * mix,
            right * (1.0 - mix) + dr * mix,
        )
    }
}

// ---------------------------------------------------------------------------
// Chorus (stereo modulated delay)
// ---------------------------------------------------------------------------

pub struct Chorus {
    delay_l: DelayLine,
    delay_r: DelayLine,
    lfo_phase: f32,
    sample_rate: f32,
}

impl Chorus {
    pub fn new(sample_rate: f32) -> Self {
        // Max delay ~20ms
        let max_samples = (sample_rate * 0.02) as usize + 256;
        Self {
            delay_l: DelayLine::new(max_samples),
            delay_r: DelayLine::new(max_samples),
            lfo_phase: 0.0,
            sample_rate,
        }
    }

    pub fn reset(&mut self) {
        self.delay_l.clear();
        self.delay_r.clear();
        self.lfo_phase = 0.0;
    }

    pub fn process(
        &mut self,
        left: f32,
        right: f32,
        rate_hz: f32,
        depth: f32,
        mix: f32,
    ) -> (f32, f32) {
        let base_delay = 0.007 * self.sample_rate; // 7ms
        let mod_range = 0.003 * self.sample_rate * depth; // up to 3ms

        let lfo_l = (self.lfo_phase * std::f32::consts::TAU).sin();
        let lfo_r = ((self.lfo_phase + 0.25) * std::f32::consts::TAU).sin(); // 90 deg offset

        let delay_l = base_delay + lfo_l * mod_range;
        let delay_r = base_delay + lfo_r * mod_range;

        self.delay_l.push(left);
        self.delay_r.push(right);

        let wet_l = self.delay_l.tap_linear(delay_l);
        let wet_r = self.delay_r.tap_linear(delay_r);

        self.lfo_phase += rate_hz / self.sample_rate;
        self.lfo_phase -= self.lfo_phase.floor();

        (
            left * (1.0 - mix) + wet_l * mix,
            right * (1.0 - mix) + wet_r * mix,
        )
    }
}

// ---------------------------------------------------------------------------
// Stereo Delay
// ---------------------------------------------------------------------------

pub struct StereoDelay {
    delay_l: DelayLine,
    delay_r: DelayLine,
    damping_l: OnePole,
    damping_r: OnePole,
    sample_rate: f32,
}

impl StereoDelay {
    pub fn new(sample_rate: f32) -> Self {
        // Max 2 seconds
        let max_samples = (sample_rate * 2.0) as usize + 256;
        let mut damping_l = OnePole::new();
        let mut damping_r = OnePole::new();
        damping_l.set_cutoff(8000.0, sample_rate);
        damping_r.set_cutoff(8000.0, sample_rate);

        Self {
            delay_l: DelayLine::new(max_samples),
            delay_r: DelayLine::new(max_samples),
            damping_l,
            damping_r,
            sample_rate,
        }
    }

    pub fn reset(&mut self) {
        self.delay_l.clear();
        self.delay_r.clear();
        self.damping_l.clear();
        self.damping_r.clear();
    }

    pub fn process(
        &mut self,
        left: f32,
        right: f32,
        time_l_ms: f32,
        time_r_ms: f32,
        feedback: f32,
        mix: f32,
    ) -> (f32, f32) {
        let delay_l_samp = time_l_ms * 0.001 * self.sample_rate;
        let delay_r_samp = time_r_ms * 0.001 * self.sample_rate;

        let wet_l = self.delay_l.tap_linear(delay_l_samp);
        let wet_r = self.delay_r.tap_linear(delay_r_samp);

        let fb_l = self.damping_l.process(wet_l) * feedback;
        let fb_r = self.damping_r.process(wet_r) * feedback;

        self.delay_l.push(left + fb_l);
        self.delay_r.push(right + fb_r);

        (
            left * (1.0 - mix) + wet_l * mix,
            right * (1.0 - mix) + wet_r * mix,
        )
    }
}
