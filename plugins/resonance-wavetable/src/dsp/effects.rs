/// Post-voice effects: distortion, chorus, stereo delay.
use resonance_dsp::{tanh_fast, Biquad, DelayLine, Lfo, OnePole, SimpleRng};

// ---------------------------------------------------------------------------
// Distortion (tanh soft-clip waveshaper)
// ---------------------------------------------------------------------------

pub struct Distortion;

impl Distortion {
    /// Process a stereo pair through distortion.
    ///
    /// `tanh_fast` rather than `f32::tanh` — two libm calls per sample on the
    /// master path. See [`resonance_dsp::tanh_fast`] for the error bound.
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
