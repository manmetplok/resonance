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
    /// `curve` ranges from -1..1:
    ///   -1 = very convex (snappy attack, slow tail)
    ///    0 = natural exponential
    ///   +1 = very concave (slow attack, snappy tail)
    pub fn next(
        &mut self,
        attack_s: f32,
        decay_s: f32,
        sustain: f32,
        release_s: f32,
        curve: f32,
    ) -> f32 {
        match self.stage {
            EnvStage::Idle => 0.0,
            EnvStage::Attack => {
                let coeff = self.exp_coeff(attack_s, curve);
                // Overshoot target so exponential actually reaches 1.0
                self.level += coeff * (1.3 - self.level);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = EnvStage::Decay;
                }
                self.level
            }
            EnvStage::Decay => {
                let coeff = self.exp_coeff(decay_s, -curve);
                // Target slightly below sustain
                let target = sustain - 0.001;
                self.level += coeff * (target - self.level);
                if self.level <= sustain + 0.0001 {
                    self.level = sustain;
                    self.stage = EnvStage::Sustain;
                }
                self.level
            }
            EnvStage::Sustain => {
                self.level = sustain;
                sustain
            }
            EnvStage::Release => {
                let coeff = self.exp_coeff(release_s, -curve);
                self.level += coeff * (-0.001 - self.level);
                if self.level <= 0.0001 {
                    self.level = 0.0;
                    self.stage = EnvStage::Idle;
                }
                self.level
            }
        }
    }

    /// Compute per-sample exponential coefficient from time and curve shape.
    fn exp_coeff(&self, time_s: f32, curve: f32) -> f32 {
        // Shape factor: curve=0 -> factor=1.0, curve=-1 -> 0.2 (fast), curve=+1 -> 5.0 (slow)
        let shape = (1.0 + curve * 0.8).max(0.2);
        let samples = (time_s * self.sample_rate * shape).max(1.0);
        1.0 - (-1.0 / samples).exp()
    }
}
