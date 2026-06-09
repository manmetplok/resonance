/// ADSR envelope generator with adjustable curve shape.

#[derive(Clone, Copy, PartialEq)]
pub enum EnvStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone)]
pub struct AdsrEnvelope {
    pub stage: EnvStage,
    pub level: f32,
    sample_rate: f32,
}

/// Block-rate exponential coefficients for the three timed stages.
/// Computed once per audio block by [`EnvCoeffs::for_params`] (the
/// `1.0 - (-1.0 / samples).exp()` is the expensive part) and reused
/// per-sample inside the voice loop, replacing the old per-sample
/// `.exp()` calls that compounded across 32 voices × 2 envelopes.
#[derive(Clone, Copy)]
pub struct EnvCoeffs {
    pub attack: f32,
    pub decay: f32,
    pub release: f32,
    pub sustain: f32,
}

impl EnvCoeffs {
    /// Build coefficients from snapshot params. Times and curve are
    /// constant for the whole block, so this only runs once at the top
    /// of `render_block` per envelope.
    #[inline]
    pub fn for_params(
        attack_s: f32,
        decay_s: f32,
        sustain: f32,
        release_s: f32,
        curve: f32,
        sample_rate: f32,
    ) -> Self {
        Self {
            attack: exp_coeff(attack_s, curve, sample_rate),
            decay: exp_coeff(decay_s, -curve, sample_rate),
            release: exp_coeff(release_s, -curve, sample_rate),
            sustain,
        }
    }
}

impl AdsrEnvelope {
    pub fn new() -> Self {
        Self {
            stage: EnvStage::Idle,
            level: 0.0,
            sample_rate: 44100.0,
        }
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sample_rate = sr;
    }

    pub fn trigger(&mut self) {
        self.stage = EnvStage::Attack;
        // Don't reset level -- allows re-triggering from current position
    }

    pub fn release(&mut self) {
        if self.stage != EnvStage::Idle {
            self.stage = EnvStage::Release;
        }
    }

    pub fn reset(&mut self) {
        self.stage = EnvStage::Idle;
        self.level = 0.0;
    }

    pub fn is_idle(&self) -> bool {
        self.stage == EnvStage::Idle
    }

    /// Advance one sample. Returns envelope value in 0..1.
    ///
    /// All four coefficients in `c` were computed once at the top of
    /// the audio block by `EnvCoeffs::for_params`; this routine is now
    /// branchy add/multiply only — no `.exp()` per sample.
    #[inline]
    pub fn next(&mut self, c: &EnvCoeffs) -> f32 {
        match self.stage {
            EnvStage::Idle => 0.0,
            EnvStage::Attack => {
                // Overshoot target so exponential actually reaches 1.0
                self.level += c.attack * (1.3 - self.level);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = EnvStage::Decay;
                }
                self.level
            }
            EnvStage::Decay => {
                // Target slightly below sustain
                let target = c.sustain - 0.001;
                self.level += c.decay * (target - self.level);
                if self.level <= c.sustain + 0.0001 {
                    self.level = c.sustain;
                    self.stage = EnvStage::Sustain;
                }
                self.level
            }
            EnvStage::Sustain => {
                self.level = c.sustain;
                c.sustain
            }
            EnvStage::Release => {
                self.level += c.release * (-0.001 - self.level);
                if self.level <= 0.0001 {
                    self.level = 0.0;
                    self.stage = EnvStage::Idle;
                }
                self.level
            }
        }
    }
}

/// Per-sample exponential coefficient. The `.exp()` is what we hoisted
/// out of the per-sample loop; this is now called O(1) per audio block
/// per envelope per voice instead of O(blocksize).
#[inline]
fn exp_coeff(time_s: f32, curve: f32, sample_rate: f32) -> f32 {
    // Shape factor: curve=0 -> factor=1.0, curve=-1 -> 0.2 (fast), curve=+1 -> 5.0 (slow)
    let shape = (1.0 + curve * 0.8).max(0.2);
    let samples = (time_s * sample_rate * shape).max(1.0);
    1.0 - (-1.0 / samples).exp()
}

impl Default for AdsrEnvelope {
    fn default() -> Self {
        Self::new()
    }
}
