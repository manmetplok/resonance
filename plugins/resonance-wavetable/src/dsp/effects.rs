/// Post-voice effects: distortion, chorus, stereo delay — plus the per-voice
/// pre-filter drive, which lives here beside the master distortion it is
/// the other half of.
use resonance_dsp::{tanh_fast, DcBlocker, DelayLine, OnePole, OversampleFactor, Oversampler};

// ---------------------------------------------------------------------------
// Distortion (tanh soft-clip waveshaper)
// ---------------------------------------------------------------------------

pub struct Distortion;

impl Distortion {
    /// Process a stereo pair through distortion.
    ///
    /// `tanh_fast` rather than `f32::tanh` — two libm calls per sample on the
    /// master path. See [`resonance_dsp::tanh_fast`] for the error bound.
    ///
    /// This is the original stage and is still what [`DistortionStage`] runs
    /// for `Soft` mode with oversampling, tone and auto gain all off — the
    /// default — so every patch saved before the other modes existed renders
    /// bit-identically.
    #[inline]
    pub fn process(left: f32, right: f32, drive: f32, mix: f32) -> (f32, f32) {
        let dl = tanh_fast(left * drive);
        let dr = tanh_fast(right * drive);
        (
            left * (1.0 - mix) + dl * mix,
            right * (1.0 - mix) + dr * mix,
        )
    }
}

/// The master distortion's waveshaper curves. The integer values are the
/// `dist_mode` parameter's and are persisted in presets — append, never
/// reorder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum DistMode {
    /// Symmetric `tanh` — odd harmonics only. The original curve.
    Soft = 0,
    /// `tanh` of a biased input: asymmetric, so it adds even harmonics the
    /// way a single-ended valve stage does. The bias also puts a DC offset
    /// on the wet signal, which a DC blocker removes.
    Tube = 1,
    /// Wavefolder: past ±1 the signal folds back instead of flattening, so
    /// rising drive adds ever-higher partials rather than just squaring off.
    Fold = 2,
    /// Hard clip at ±1.
    Hard = 3,
    /// Bit-depth quantiser plus sample-and-hold rate reduction.
    Crush = 4,
    /// Full-wave rectifier into a soft clip: an octave-up, buzzy fuzz. DC
    /// blocked like `Tube`.
    Rectify = 5,
}

impl DistMode {
    /// Display labels, indexed by discriminant.
    pub const LABELS: [&'static str; 6] = ["Soft", "Tube", "Fold", "Hard", "Crush", "Rectify"];

    pub fn from_int(v: i32) -> Self {
        match v {
            1 => Self::Tube,
            2 => Self::Fold,
            3 => Self::Hard,
            4 => Self::Crush,
            5 => Self::Rectify,
            _ => Self::Soft,
        }
    }

    /// The modes whose curve is asymmetric and so leaves DC on the wet
    /// signal.
    fn needs_dc_block(self) -> bool {
        matches!(self, Self::Tube | Self::Rectify)
    }
}

/// `dist_tone` at or above this is "open": the tone filter is skipped
/// entirely, which is what keeps its default sound-neutral. (A one-pole at
/// 20 kHz is not transparent — it is several dB down at 20 kHz.)
pub const TONE_OPEN_HZ: f32 = 20_000.0;

/// Input bias of the `Tube` curve. `tanh(x + 0.5) − tanh(0.5)`: small-
/// signal slope sech²(0.5) ≈ 0.79, positive swings saturate at ≈ 0.54,
/// negative ones at ≈ −1.46 — the lopsided transfer that makes the second
/// harmonic.
const TUBE_BIAS: f32 = 0.5;

/// The level auto gain normalises around: a −12 dBFS peak (0.25) comes out
/// of the stage at the level it went in, whatever the drive. Louder input
/// still gets louder, just not by the drive's full gain.
const AUTO_GAIN_REF: f32 = 0.25;

/// Block-constant settings for [`DistortionStage`], read from the parameter
/// snapshot once per block.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DistSettings {
    pub mode: DistMode,
    pub oversample: OversampleFactor,
    /// Tone low-pass cutoff in Hz; `>= TONE_OPEN_HZ` bypasses it.
    pub tone_hz: f32,
    pub auto_gain: bool,
    /// `Crush` quantiser depth (1..16, fractional).
    pub bits: f32,
    /// `Crush` sample-and-hold rate in Hz.
    pub crush_rate: f32,
}

impl Default for DistSettings {
    /// The parameter defaults — the configuration that renders exactly as
    /// the original [`Distortion`].
    fn default() -> Self {
        Self {
            mode: DistMode::Soft,
            oversample: OversampleFactor::Off,
            tone_hz: TONE_OPEN_HZ,
            auto_gain: false,
            bits: 8.0,
            crush_rate: 11025.0,
        }
    }
}

/// The master distortion stage: mode, optional 2×/4× oversampling, a tone
/// filter and auto gain around the [`Distortion`] mix.
///
/// # Oversampling and latency
///
/// The oversampler is `resonance_dsp`'s polyphase IIR half-band pair, not a
/// FIR, so it adds **no fixed latency** and the plugin reports none. What
/// it does add is a small frequency-dependent group delay — about 4 samples
/// at low frequencies for 2×, 5.5 for 4×, at any base rate (≈ 0.1 ms at
/// 48 kHz; pinned in `resonance-dsp/tests/oversample.rs`). Reporting that
/// would buy nothing: a latency change needs a plugin restart from the
/// host, for a tenth of a millisecond on an instrument whose output is not
/// time-aligned against an input anyway. What *would* be audible is the
/// dry path skipping that delay while the wet path takes it — a comb
/// filter on every `mix < 1` — so the dry/wet blend happens inside the
/// oversampled loop, and dry and wet leave through the same downsampler.
///
/// # RT safety
///
/// Everything is inline, fixed-size state: the half-band coefficients are
/// designed in [`DistortionStage::new`], which the engine calls from
/// `initialize`. Changing the oversampling factor at run time only clears
/// filter state.
pub struct DistortionStage {
    os: [Oversampler; 2],
    dc: [DcBlocker; 2],
    tone: [OnePole; 2],
    /// Sample-and-hold value and phase per channel (`Crush`).
    hold: [f32; 2],
    hold_phase: [f32; 2],

    sample_rate: f32,
    settings: DistSettings,
    /// True when the settings are exactly the original stage's, and
    /// [`DistortionStage::process`] defers to [`Distortion::process`].
    legacy: bool,
    tone_on: bool,
    /// `tanh_fast(TUBE_BIAS)`, subtracted so the `Tube` curve passes
    /// through the origin.
    tube_offset: f32,
    /// Quantiser steps per unit (`2^(bits−1)`) and the sample-and-hold
    /// phase increment per stage-rate sample.
    crush_steps: f32,
    crush_inc: f32,
}

impl DistortionStage {
    pub fn new(sample_rate: f32) -> Self {
        let mut stage = Self {
            os: [Oversampler::new(), Oversampler::new()],
            dc: [DcBlocker::default(), DcBlocker::default()],
            tone: [OnePole::new(), OnePole::new()],
            hold: [0.0; 2],
            hold_phase: [1.0; 2],
            sample_rate,
            settings: DistSettings::default(),
            legacy: true,
            tone_on: false,
            tube_offset: tanh_fast(TUBE_BIAS),
            crush_steps: 1.0,
            crush_inc: 1.0,
        };
        stage.derive();
        stage
    }

    pub fn reset(&mut self) {
        for os in &mut self.os {
            os.reset();
        }
        for dc in &mut self.dc {
            dc.reset();
        }
        for t in &mut self.tone {
            t.clear();
        }
        self.hold = [0.0; 2];
        self.hold_phase = [1.0; 2];
    }

    /// Apply this block's settings. Cheap and RT-safe; call once per block.
    /// A change of oversampling factor clears the filter state (it belongs
    /// to a different rate); anything else keeps it.
    pub fn configure(&mut self, settings: DistSettings) {
        if settings == self.settings {
            return;
        }
        let rate_changed = settings.oversample != self.settings.oversample;
        self.settings = settings;
        if rate_changed {
            self.reset();
        }
        self.derive();
    }

    /// Recompute everything that depends on the settings and the rate.
    fn derive(&mut self) {
        let s = self.settings;
        for os in &mut self.os {
            os.set_factor(s.oversample);
        }
        let stage_rate = self.sample_rate * s.oversample.ratio() as f32;
        for dc in &mut self.dc {
            dc.set_cutoff(DcBlocker::DEFAULT_CUTOFF_HZ, stage_rate);
        }
        self.tone_on = s.tone_hz < TONE_OPEN_HZ;
        for t in &mut self.tone {
            t.set_cutoff(s.tone_hz, stage_rate);
        }
        self.crush_steps = (s.bits.clamp(1.0, 16.0) - 1.0).exp2();
        self.crush_inc = (s.crush_rate / stage_rate).clamp(0.0, 1.0);
        self.legacy = s.mode == DistMode::Soft
            && s.oversample == OversampleFactor::Off
            && !self.tone_on
            && !s.auto_gain;
    }

    /// Process one stereo sample. `drive` is the (smoothed, modulated)
    /// drive, `mix` the dry/wet blend.
    #[inline]
    pub fn process(&mut self, left: f32, right: f32, drive: f32, mix: f32) -> (f32, f32) {
        if self.legacy {
            return Distortion::process(left, right, drive, mix);
        }
        let makeup = if self.settings.auto_gain {
            AUTO_GAIN_REF / tanh_fast(AUTO_GAIN_REF * drive)
        } else {
            1.0
        };
        let wet_gain = makeup * mix;
        let dry_gain = 1.0 - mix;
        (
            self.channel(0, left, drive, dry_gain, wet_gain),
            self.channel(1, right, drive, dry_gain, wet_gain),
        )
    }

    #[inline]
    fn channel(&mut self, ch: usize, x: f32, drive: f32, dry_gain: f32, wet_gain: f32) -> f32 {
        let ratio = self.os[ch].ratio();
        let dc_block = self.settings.mode.needs_dc_block();
        let mut buf = self.os[ch].upsample(x);
        for i in 0..ratio {
            let dry = buf[i];
            let mut wet = self.shape(ch, dry * drive);
            if dc_block {
                wet = self.dc[ch].process(wet);
            }
            if self.tone_on {
                wet = self.tone[ch].process(wet);
            }
            buf[i] = dry * dry_gain + wet * wet_gain;
        }
        self.os[ch].downsample(&buf)
    }

    /// The mode's transfer curve, at the (possibly oversampled) stage rate.
    #[inline]
    fn shape(&mut self, ch: usize, u: f32) -> f32 {
        match self.settings.mode {
            DistMode::Soft => tanh_fast(u),
            DistMode::Tube => tanh_fast(u + TUBE_BIAS) - self.tube_offset,
            DistMode::Fold => fold(u),
            DistMode::Hard => u.clamp(-1.0, 1.0),
            DistMode::Crush => {
                // Sample-and-hold at `crush_rate`, quantising each newly
                // held sample to the bit depth.
                self.hold_phase[ch] += self.crush_inc;
                if self.hold_phase[ch] >= 1.0 {
                    self.hold_phase[ch] -= self.hold_phase[ch].floor();
                    let q = self.crush_steps;
                    self.hold[ch] = (u.clamp(-1.0, 1.0) * q).round() / q;
                }
                self.hold[ch]
            }
            DistMode::Rectify => tanh_fast(u.abs()),
        }
    }
}

/// Wavefolder: a triangle fold (slope 1 through the origin, turning back at
/// every odd integer) rounded by the cubic `t·(1.5 − 0.5·t²)`, which is flat
/// at ±1. The rounding gives the fold points the soft peaks of a sine fold
/// without a `sin` per sample.
#[inline]
fn fold(u: f32) -> f32 {
    let t = ((u - 1.0).rem_euclid(4.0) - 2.0).abs() - 1.0;
    t * (1.5 - 0.5 * t * t)
}

// ---------------------------------------------------------------------------
// Per-voice pre-filter drive
// ---------------------------------------------------------------------------

/// Gain into the per-voice shaper at `voice_drive` = 1 is `1 + this`.
const VOICE_DRIVE_GAIN: f32 = 11.0;

/// Saturate one voice's stereo pair before its filter. `amount` is the
/// resolved `voice_drive` (param + modulation), in (0, 1]; the caller skips
/// the call entirely at 0, which is what keeps the default bit-identical.
///
/// The shaped signal is `tanh(g·x)` with `g = 1 + 11·amount`, crossfaded in
/// by `amount` itself, so the result is continuous from the clean signal at
/// 0, a blend of clean and gently driven at low settings, and all
/// `tanh(12x)` — hard saturation at any playing level — at 1. Two
/// `tanh_fast` per voice per sample, no state and no oversampling: before
/// the filter, which removes much of what would alias, it is the cheap half
/// of the synth's drive, and the master stage is where the expensive half
/// lives.
#[inline]
pub fn voice_saturate(left: f32, right: f32, amount: f32) -> (f32, f32) {
    let g = 1.0 + VOICE_DRIVE_GAIN * amount;
    let sl = tanh_fast(left * g);
    let sr = tanh_fast(right * g);
    (left + (sl - left) * amount, right + (sr - right) * amount)
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
        }
    }

    pub fn reset(&mut self) {
        self.delay_l.clear();
        self.delay_r.clear();
        self.damping_l.clear();
        self.damping_r.clear();
    }

    /// Delay times are taken in *samples* (fractional): the engine smooths
    /// the resolved sample count so an automated time change glides the
    /// read tap instead of relocating it discontinuously.
    pub fn process(
        &mut self,
        left: f32,
        right: f32,
        delay_l_samp: f32,
        delay_r_samp: f32,
        feedback: f32,
        mix: f32,
    ) -> (f32, f32) {
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
