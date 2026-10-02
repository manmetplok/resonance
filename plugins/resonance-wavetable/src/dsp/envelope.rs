/// ADSR envelope generator with adjustable curve shape.
///
/// Every stage is a one-pole glide toward a target just past where the
/// stage ends, so it arrives in finite time. The labelled times are
/// *times to target* (DSP2-12), not time constants:
///
/// - **Attack**: from 0 to the peak (1.0).
/// - **Decay**: from the peak to the sustain level, whatever it is.
/// - **Release**: from full scale to −60 dB. A release from a lower level
///   gets there sooner, as on an analog ADSR; the voice goes idle at −80 dB,
///   2-24% after the labelled time depending on the curve (10% at 0).
///
/// The curve changes each stage's *shape* only, by moving that overshoot
/// target: a far target makes the glide close to linear, a near one makes
/// it strongly exponential. Positive curve gives a more linear attack and a
/// more exponential (snappier) decay and release; negative the reverse.
/// The coefficient is then solved so the stage still lands on its label.
/// (Before DSP2-12 the knob scaled the time constant by `1 + 0.8·curve`,
/// and the times were time constants: "Attack 100 ms" peaked at 147 ms and
/// "Release 10 s" held a voice for ~68 s.)

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

/// Where a decay or release counts as arrived: within this of its level.
const END_THRESHOLD: f32 = 1.0e-4;
/// Release labels are time to −60 dB.
const RELEASE_FLOOR: f32 = 1.0e-3;
/// Attack overshoot past the peak at curve 0 (the pre-DSP2-12 shape).
const ATTACK_OVERSHOOT: f32 = 0.3;
/// Decay/release undershoot past their end at curve 0 (likewise).
const FALL_UNDERSHOOT: f32 = 1.0e-3;
/// Sustain changes glide with this time constant instead of stepping.
const SUSTAIN_SMOOTH_S: f32 = 0.005;

/// Block-rate coefficients for the three timed stages and the sustain
/// glide. Computed once per audio block by [`EnvCoeffs::for_params`] (the
/// `exp`/`ln` are the expensive part) and reused per sample inside the
/// voice loop.
#[derive(Clone, Copy)]
pub struct EnvCoeffs {
    pub attack: f32,
    pub decay: f32,
    pub release: f32,
    pub sustain: f32,
    /// Attack target: `1 + overshoot`.
    pub attack_target: f32,
    /// How far below sustain (decay) or zero (release) the fall aims.
    pub decay_undershoot: f32,
    pub release_undershoot: f32,
    /// One-pole coefficient of the sustain glide.
    pub sustain_smooth: f32,
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
        let curve = curve.clamp(-1.0, 1.0);
        let sustain = sustain.clamp(0.0, 1.0);
        // Attack: from 0 toward `1 + o`, arriving at 1 after
        // `τ·ln((1 + o) / o)`.
        let o = ATTACK_OVERSHOOT * 4f32.powf(curve);
        let attack = coeff_for(attack_s, ((1.0 + o) / o).ln(), sample_rate);
        // Decay/release: the fall aims `u` past its end and arrives within
        // the threshold. Positive curve = smaller `u` = more exponential.
        let ud = FALL_UNDERSHOOT * 8f32.powf(-curve);
        let span = 1.0 - sustain;
        let decay = if span <= END_THRESHOLD {
            1.0
        } else {
            coeff_for(decay_s, ((span + ud) / (END_THRESHOLD + ud)).ln(), sample_rate)
        };
        let ur = ud;
        let release = coeff_for(release_s, ((1.0 + ur) / (RELEASE_FLOOR + ur)).ln(), sample_rate);
        Self {
            attack,
            decay,
            release,
            sustain,
            attack_target: 1.0 + o,
            decay_undershoot: ud,
            release_undershoot: ur,
            sustain_smooth: 1.0 - (-1.0 / (SUSTAIN_SMOOTH_S * sample_rate).max(1.0)).exp(),
        }
    }
}

/// One-pole coefficient whose glide covers `ln_ratio` time constants in
/// `time_s`: `τ = time·fs / ln_ratio` samples.
#[inline]
fn coeff_for(time_s: f32, ln_ratio: f32, sample_rate: f32) -> f32 {
    let tau = (time_s * sample_rate / ln_ratio.max(1.0e-6)).max(1.0e-3);
    1.0 - (-1.0 / tau).exp()
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
    /// All coefficients in `c` were computed once at the top of the audio
    /// block by `EnvCoeffs::for_params`; this routine is branchy
    /// add/multiply only — no `.exp()` per sample.
    #[inline]
    pub fn next(&mut self, c: &EnvCoeffs) -> f32 {
        match self.stage {
            EnvStage::Idle => 0.0,
            EnvStage::Attack => {
                self.level += c.attack * (c.attack_target - self.level);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = EnvStage::Decay;
                }
                self.level
            }
            EnvStage::Decay => {
                let target = c.sustain - c.decay_undershoot;
                self.level += c.decay * (target - self.level);
                if self.level <= c.sustain + END_THRESHOLD {
                    // Within the threshold (or below a sustain raised
                    // mid-decay): the sustain glide takes it from here,
                    // so this hand-over never steps.
                    self.stage = EnvStage::Sustain;
                }
                self.level
            }
            EnvStage::Sustain => {
                // Glide rather than snap: the sustain value is a block
                // snapshot, so automating it would otherwise step.
                self.level += c.sustain_smooth * (c.sustain - self.level);
                self.level
            }
            EnvStage::Release => {
                self.level += c.release * (-c.release_undershoot - self.level);
                if self.level <= END_THRESHOLD {
                    self.level = 0.0;
                    self.stage = EnvStage::Idle;
                }
                self.level
            }
        }
    }
}

impl Default for AdsrEnvelope {
    fn default() -> Self {
        Self::new()
    }
}
