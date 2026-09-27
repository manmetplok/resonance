/// Post-voice effects: distortion, chorus, stereo delay — plus the per-voice
/// pre-filter drive, which lives here beside the master distortion it is
/// the other half of.
use resonance_dsp::{
    tanh_fast, Biquad, DcBlocker, DelayLine, Lfo, OnePole, OversampleFactor, Oversampler,
    SimpleRng,
};

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
// Chorus (stereo modulated delay, plus BBD-style Juno / ensemble modes)
// ---------------------------------------------------------------------------

/// Which chorus circuit [`Chorus`] models. Discriminants are the values of
/// the `chorus_mode` parameter, so they are append-only.
///
/// `Classic` is the original sine-LFO stereo chorus and stays the default:
/// its output is bit-identical to the chorus this synth shipped with. The
/// other four share one mono "bucket-brigade" line with band-limiting
/// pre/post filters, gentle saturation and optional hiss (see
/// [`Chorus::feed_bbd`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChorusMode {
    Classic = 0,
    JunoI = 1,
    JunoII = 2,
    JunoBoth = 3,
    Ensemble = 4,
}

/// Rate and delay swing of one Juno-60 chorus setting.
struct JunoSpec {
    rate_hz: f32,
    min_ms: f32,
    max_ms: f32,
}

/// The Juno-60's three chorus settings, as measured from a hardware unit
/// by Andy Harman (pendragon-andyh/Juno60, `Chorus/README.md`) — the
/// figures most emulations cite. All three drive the two BBD lines from
/// one triangle LFO, the right line's modulation inverted (180°):
///
/// | setting | LFO rate | delay swing     |
/// |---------|----------|-----------------|
/// | I       | 0.513 Hz | 1.66 – 5.35 ms  |
/// | II      | 0.863 Hz | 1.66 – 5.35 ms  |
/// | I+II    | 9.75 Hz  | 3.3 – 3.7 ms    |
///
/// I+II is the fast, shallow, vibrato-like setting (both buttons down).
const JUNO_I: JunoSpec = JunoSpec {
    rate_hz: 0.513,
    min_ms: 1.66,
    max_ms: 5.35,
};
const JUNO_II: JunoSpec = JunoSpec {
    rate_hz: 0.863,
    min_ms: 1.66,
    max_ms: 5.35,
};
const JUNO_I_II: JunoSpec = JunoSpec {
    rate_hz: 9.75,
    min_ms: 3.3,
    max_ms: 3.7,
};

/// Ensemble (Solina-style string ensemble): three taps off the one BBD
/// line, each modulated by a slow chorus LFO plus a fast vibrato LFO, the
/// three taps 120° apart on both. Approximate values after the usual
/// descriptions of the Solina / ARP string-ensemble circuit (slow LFO well
/// under 1 Hz, fast ≈ 6 Hz, a few ms of delay); the slow rate is the
/// Rate knob here, so only the fast one is fixed.
const ENSEMBLE_CENTRE_MS: f32 = 5.0;
const ENSEMBLE_SLOW_SWING_MS: f32 = 1.5;
const ENSEMBLE_FAST_SWING_MS: f32 = 0.2;
const ENSEMBLE_FAST_HZ: f32 = 6.0;

/// Corner of the BBD anti-alias (pre) and reconstruction (post) lowpasses.
/// The Juno-60 puts a 12 dB/oct lowpass in front of its BBDs (Harman,
/// ibid.) and another behind them; together they are what takes the fizz
/// off the wet signal. Two 2-pole Butterworth sections here give 24 dB/oct
/// above the corner — ≈ −12 dB at 12 kHz, ≈ −19 dB at 15 kHz.
const BBD_LOWPASS_HZ: f32 = 9000.0;
const BBD_LOWPASS_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Soft-clip drive of the BBD stage. `tanh(k·x)/k` is unity-gain for small
/// signals and only rounds off peaks near full scale (−1.3 dB at 0 dBFS) —
/// the gentle compression of a bucket-brigade running out of headroom.
const BBD_SAT_DRIVE: f32 = 0.7;

/// Peak hiss level at Noise = 100 % (−40 dBFS), injected into the BBD line
/// so the post filter band-limits it like the real thing's clock noise.
const BBD_NOISE_MAX: f32 = 0.01;

/// Length of each half of the mode-switch crossfade (wet out, then in).
const MODE_FADE_MS: f32 = 10.0;

impl ChorusMode {
    pub const DEFAULT: ChorusMode = ChorusMode::Classic;

    /// Choice labels of the `chorus_mode` parameter, indexed by discriminant.
    pub const LABELS: [&'static str; 5] = ["Classic", "I", "II", "I+II", "Ens"];

    pub fn from_int(v: i32) -> Self {
        match v {
            1 => ChorusMode::JunoI,
            2 => ChorusMode::JunoII,
            3 => ChorusMode::JunoBoth,
            4 => ChorusMode::Ensemble,
            _ => ChorusMode::Classic,
        }
    }

    pub fn label(self) -> &'static str {
        Self::LABELS[self as usize]
    }

    /// Whether the Rate parameter does anything in this mode. The three
    /// Juno settings run at the hardware's fixed rates, so the editor
    /// swaps the Rate knob for [`Self::fixed_rate_label`] there.
    pub fn uses_rate(self) -> bool {
        matches!(self, ChorusMode::Classic | ChorusMode::Ensemble)
    }

    /// Whether this mode runs through the BBD line (and so reads Noise).
    pub fn is_bbd(self) -> bool {
        self != ChorusMode::Classic
    }

    /// The fixed LFO rate a Juno mode runs at, for display in place of the
    /// Rate knob. `None` for the modes that use the Rate parameter.
    pub fn fixed_rate_label(self) -> Option<&'static str> {
        match self {
            ChorusMode::JunoI => Some("0.51 Hz"),
            ChorusMode::JunoII => Some("0.86 Hz"),
            ChorusMode::JunoBoth => Some("9.75 Hz"),
            _ => None,
        }
    }

    fn juno_spec(self) -> Option<&'static JunoSpec> {
        match self {
            ChorusMode::JunoI => Some(&JUNO_I),
            ChorusMode::JunoII => Some(&JUNO_II),
            ChorusMode::JunoBoth => Some(&JUNO_I_II),
            _ => None,
        }
    }
}

pub struct Chorus {
    delay_l: DelayLine,
    delay_r: DelayLine,
    lfo_phase: f32,
    sample_rate: f32,

    /// The mono bucket-brigade line every non-Classic mode taps. Fed on
    /// every sample, whatever the mode, so a switch never reads stale audio.
    bbd: DelayLine,
    bbd_pre: [Biquad; 2],
    bbd_post_l: [Biquad; 2],
    bbd_post_r: [Biquad; 2],
    /// Juno triangle LFO phase, 0..1.
    juno_phase: f32,
    /// Ensemble slow / fast LFOs, one per tap, 120° apart.
    ens_slow: [Lfo; 3],
    ens_fast: [Lfo; 3],
    rng: SimpleRng,

    /// The mode whose wet signal is currently audible, and the gain the
    /// wet path is at while a switch fades it out and the new one in.
    active: ChorusMode,
    fade: f32,
    fade_step: f32,
}

/// Triangle in -1..1 from a 0..1 phase (+1 at phase 0, -1 at 0.5).
#[inline]
fn triangle(phase: f32) -> f32 {
    4.0 * (phase - 0.5).abs() - 1.0
}

fn bbd_lowpass(sample_rate: f32) -> [Biquad; 2] {
    let mut f = [Biquad::identity(); 2];
    for b in &mut f {
        b.set_low_pass(sample_rate, BBD_LOWPASS_HZ, BBD_LOWPASS_Q);
    }
    f
}

#[inline]
fn cascade(filters: &mut [Biquad; 2], x: f32) -> f32 {
    let y = filters[0].process(x);
    filters[1].process(y)
}

impl Chorus {
    pub fn new(sample_rate: f32) -> Self {
        // Max delay ~20ms
        let max_samples = (sample_rate * 0.02) as usize + 256;
        let third = 1.0 / 3.0;
        Self {
            delay_l: DelayLine::new(max_samples),
            delay_r: DelayLine::new(max_samples),
            lfo_phase: 0.0,
            sample_rate,
            bbd: DelayLine::new(max_samples),
            bbd_pre: bbd_lowpass(sample_rate),
            bbd_post_l: bbd_lowpass(sample_rate),
            bbd_post_r: bbd_lowpass(sample_rate),
            juno_phase: 0.0,
            ens_slow: [0.0, third, 2.0 * third].map(|p| Lfo::new(1.0, sample_rate, p)),
            ens_fast: [0.0, third, 2.0 * third]
                .map(|p| Lfo::new(ENSEMBLE_FAST_HZ, sample_rate, p)),
            rng: SimpleRng::new(0x4a55_4e4f),
            active: ChorusMode::DEFAULT,
            fade: 1.0,
            fade_step: 1.0 / (MODE_FADE_MS * 0.001 * sample_rate).max(1.0),
        }
    }

    pub fn reset(&mut self) {
        self.delay_l.clear();
        self.delay_r.clear();
        self.lfo_phase = 0.0;
        self.bbd.clear();
        for b in self
            .bbd_pre
            .iter_mut()
            .chain(&mut self.bbd_post_l)
            .chain(&mut self.bbd_post_r)
        {
            b.reset();
        }
        self.juno_phase = 0.0;
        for l in self.ens_slow.iter_mut().chain(&mut self.ens_fast) {
            l.reset();
        }
        self.rng = SimpleRng::new(0x4a55_4e4f);
        self.fade = 1.0;
    }

    /// The mode currently audible (the target of a switch still fading
    /// in counts; the one still fading out does not).
    pub fn active_mode(&self) -> ChorusMode {
        self.active
    }

    /// Adopt `mode` without a crossfade. For when the chorus is not being
    /// heard (disabled), where there is nothing to click.
    pub fn set_mode_immediate(&mut self, mode: ChorusMode) {
        self.active = mode;
        self.fade = 1.0;
    }

    /// The original Classic chorus: one sine-modulated tap per channel.
    pub fn process(
        &mut self,
        left: f32,
        right: f32,
        rate_hz: f32,
        depth: f32,
        mix: f32,
    ) -> (f32, f32) {
        let (wet_l, wet_r) = self.classic_wet(left, right, rate_hz, depth);
        (
            left * (1.0 - mix) + wet_l * mix,
            right * (1.0 - mix) + wet_r * mix,
        )
    }

    /// Process one sample in `mode`.
    ///
    /// Parameter meaning per mode:
    /// * **Classic** — exactly [`Self::process`]; `noise` is unused.
    /// * **Juno I / II / I+II** — `rate_hz` is ignored (the hardware's
    ///   rates are fixed, see [`JUNO_I`]); `depth` scales the hardware's
    ///   delay swing, 50 % being the real unit and 100 % twice it.
    /// * **Ensemble** — `rate_hz` is the slow chorus LFO; `depth` scales
    ///   both swings the same way (50 % = the nominal values).
    ///
    /// A mode change fades the wet signal out over [`MODE_FADE_MS`], swaps,
    /// and fades the new one in; the effective mix follows the fade, so
    /// the dry share rises to fill the gap instead of the level dipping.
    pub fn process_mode(
        &mut self,
        left: f32,
        right: f32,
        mode: ChorusMode,
        rate_hz: f32,
        depth: f32,
        noise: f32,
        mix: f32,
    ) -> (f32, f32) {
        if mode == ChorusMode::Classic && self.active == ChorusMode::Classic && self.fade >= 1.0
        {
            // Steady Classic: the historical path, untouched. The BBD line
            // is still fed so switching to a BBD mode later reads real audio.
            self.feed_bbd(left, right, 0.0);
            return self.process(left, right, rate_hz, depth, mix);
        }

        if mode != self.active {
            self.fade -= self.fade_step;
            if self.fade <= 0.0 {
                self.fade = 0.0;
                self.active = mode;
            }
        } else if self.fade < 1.0 {
            self.fade = (self.fade + self.fade_step).min(1.0);
        }

        let (wet_l, wet_r) = if self.active == ChorusMode::Classic {
            self.feed_bbd(left, right, 0.0);
            self.classic_wet(left, right, rate_hz, depth)
        } else {
            // Keep the Classic lines current too, for the switch back.
            self.delay_l.push(left);
            self.delay_r.push(right);
            self.feed_bbd(left, right, noise);
            self.bbd_wet(rate_hz, depth)
        };

        let wet_mix = mix * self.fade;
        (
            left * (1.0 - wet_mix) + wet_l * wet_mix,
            right * (1.0 - wet_mix) + wet_r * wet_mix,
        )
    }

    /// The Classic wet pair. Pushes the input into the Classic lines and
    /// advances the Classic LFO.
    #[inline]
    fn classic_wet(&mut self, left: f32, right: f32, rate_hz: f32, depth: f32) -> (f32, f32) {
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

        (wet_l, wet_r)
    }

    /// Write one sample into the BBD line: the input summed to mono (the
    /// Juno is a mono synth feeding a stereo chorus), anti-alias filtered,
    /// soft-saturated, plus hiss at `noise` (0..1).
    #[inline]
    fn feed_bbd(&mut self, left: f32, right: f32, noise: f32) {
        let mono = 0.5 * (left + right);
        let x = cascade(&mut self.bbd_pre, mono);
        let mut x = tanh_fast(x * BBD_SAT_DRIVE) * (1.0 / BBD_SAT_DRIVE);
        if noise > 0.0 {
            // Uniform white in -1..1.
            let white = (self.rng.next_u32() as f32) * (2.0 / u32::MAX as f32) - 1.0;
            x += white * noise * BBD_NOISE_MAX;
        }
        self.bbd.push(x);
    }

    /// Longest tap the BBD line can serve without aliasing.
    #[inline]
    fn clamp_tap(&self, samples: f32) -> f32 {
        let max = (0.02 * self.sample_rate).max(2.0);
        samples.clamp(1.0, max)
    }

    /// The wet pair of the active BBD mode, through the reconstruction
    /// filters.
    #[inline]
    fn bbd_wet(&mut self, rate_hz: f32, depth: f32) -> (f32, f32) {
        let ms = 0.001 * self.sample_rate;
        // 50 % depth = the modelled circuit's own swing.
        let depth_scale = 2.0 * depth;

        let (raw_l, raw_r) = if let Some(spec) = self.active.juno_spec() {
            let centre = 0.5 * (spec.min_ms + spec.max_ms) * ms;
            let swing = 0.5 * (spec.max_ms - spec.min_ms) * ms * depth_scale;
            let tri = triangle(self.juno_phase);
            self.juno_phase += spec.rate_hz / self.sample_rate;
            self.juno_phase -= self.juno_phase.floor();
            // Anti-phase taps: the right line's modulation is inverted.
            let dl = self.clamp_tap(centre + tri * swing);
            let dr = self.clamp_tap(centre - tri * swing);
            (self.bbd.tap_linear(dl), self.bbd.tap_linear(dr))
        } else {
            let centre = ENSEMBLE_CENTRE_MS * ms;
            let slow = ENSEMBLE_SLOW_SWING_MS * ms * depth_scale;
            let fast = ENSEMBLE_FAST_SWING_MS * ms * depth_scale;
            let mut taps = [0.0f32; 3];
            for (k, tap) in taps.iter_mut().enumerate() {
                self.ens_slow[k].set_rate(rate_hz, self.sample_rate);
                let m = self.ens_slow[k].next() * slow + self.ens_fast[k].next() * fast;
                *tap = self.bbd.tap_linear(self.clamp_tap(centre + m));
            }
            // The outer taps favour one side each, the middle one is shared.
            let third = 1.0 / 3.0;
            (
                (2.0 * taps[0] + taps[1]) * third,
                (2.0 * taps[2] + taps[1]) * third,
            )
        };

        (
            cascade(&mut self.bbd_post_l, raw_l),
            cascade(&mut self.bbd_post_r, raw_r),
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
