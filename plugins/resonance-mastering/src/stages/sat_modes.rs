//! The saturator's character modes (`sat_mode`, warmth-width-depth.md
//! §6.1 and §6.3).
//!
//! [`SatMode::Blend`], the default, is the saturator's original Tube↔Tape
//! blend, untouched (see `saturator.rs`). The other modes are voicings
//! built from `resonance_dsp`'s shared primitives — the same curves,
//! tape filters and emphasis pair the `resonance-color` plugin uses —
//! but this is the mastering stage's own signal path (decision D1: share
//! primitives, not code paths):
//!
//! | Mode | Voicing |
//! |---|---|
//! | `Tube` | [`Curve::Tube`] (bias 0.5, the wavetable Tube): H2-dominant. DC blocker after |
//! | `Tape` | A lightly biased soft curve, then a 15 ips head bump and level-dependent HF loss ([`HeadBump`], [`HfLoss`]) |
//! | `Transformer` | LF-weighted drive: a +6 dB low-shelf [`EmphasisPair`] around a slightly biased soft curve, then a sub-sonic high-pass with a small resonance and a gentle HF bell |
//! | `Console` | [`Curve::Console`], the Airwindows-style `sin` curve; its drive is part of the curve (0 dB = identity, +6 dB = the Airwindows channel) |
//! | `Warm` | [`Curve::Warm`] at full amount: even harmonics only. DC blocker after |
//! | `Inflator` | [`Curve::Inflator`], the odd density polynomial, with `sat_curve` as its Curve control: loudness without more limiting |
//!
//! # Level
//!
//! Like the Blend mode, every mode is peak-normalised: the output is
//! divided by the curve's larger magnitude at `±drive`, so a full-scale
//! input comes out near full scale at any drive and quieter material
//! gains density as the drive rises (`sat_mix` is the parallel blend).
//!
//! # Antialiasing
//!
//! The curves run through first-order ADAA ([`Adaa1`]) inside a 4× IIR
//! [`Oversampler`]. At 1× the ADAA's two-tap average would dull the top
//! octave (−3 dB at fs/4); at 4× it sits far above the audio band. (The
//! linear filters of each voicing run at the base rate, where their low
//! corners are numerically clean.)
//!
//! # Delay
//!
//! The half-band filters have no fixed delay, so the reported latency
//! does not change. They do delay the wet path by a frequency-dependent
//! amount, about 5.5 samples at 48 kHz below a few kHz, which the host's
//! delay compensation does not see. For `sat_mix` the dry signal runs
//! through an identical up/down pair, so the parallel blend does not
//! comb, and a mode is always its own group delay behind its input.
//! The stage's enable crossfade (10 ms) is the one place the delayed
//! signal meets the raw input, so it combs while it runs (first notch
//! ≈4.4 kHz at its midpoint). Matching its dry side too would make the
//! fade start and end on a step between the delayed and the raw signal
//! instead, which clicks. See the chain's module docs.
//!
//! Switching modes restarts the mode's filters; like switching the Blend
//! mode's shaper it is not crossfaded.

use resonance_dsp::tape::hf_loss_corner_hz;
use resonance_dsp::{
    Adaa1, Biquad, Curve, DcBlocker, EmphasisPair, HeadBump, HfLoss, OversampleFactor,
    Oversampler,
};

/// Which voicing the saturator runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SatMode {
    /// The original Tube↔Tape blend (`sat_character`, `sat_shaper`).
    #[default]
    Blend,
    Tube,
    Tape,
    Transformer,
    Console,
    Warm,
    Inflator,
}

impl SatMode {
    /// Display labels, indexed by [`SatMode::to_index`].
    pub const LABELS: &'static [&'static str] = &[
        "Blend",
        "Tube",
        "Tape",
        "Transformer",
        "Console",
        "Warm",
        "Inflator",
    ];

    pub fn from_index(i: i32) -> Self {
        match i {
            1 => SatMode::Tube,
            2 => SatMode::Tape,
            3 => SatMode::Transformer,
            4 => SatMode::Console,
            5 => SatMode::Warm,
            6 => SatMode::Inflator,
            _ => SatMode::Blend,
        }
    }

    pub fn to_index(self) -> i32 {
        match self {
            SatMode::Blend => 0,
            SatMode::Tube => 1,
            SatMode::Tape => 2,
            SatMode::Transformer => 3,
            SatMode::Console => 4,
            SatMode::Warm => 5,
            SatMode::Inflator => 6,
        }
    }

    /// The curve for this mode at linear `drive`, with `curve` the
    /// Inflator's Curve control (−0.5..0.5). `None` for [`SatMode::Blend`].
    pub fn curve(self, drive: f32, curve: f32) -> Option<Curve> {
        Some(match self {
            SatMode::Blend => return None,
            SatMode::Tube => Curve::Tube { bias: TUBE_BIAS },
            SatMode::Tape => Curve::Tube { bias: TAPE_BIAS },
            SatMode::Transformer => Curve::Tube {
                bias: TRANSFORMER_BIAS,
            },
            SatMode::Console => Curve::Console {
                drive: console_drive(drive),
            },
            SatMode::Warm => Curve::Warm { amount: 1.0 },
            SatMode::Inflator => Curve::Inflator { curve },
        })
    }

    /// Whether the drive is applied outside the curve (all but Console,
    /// whose drive is part of the curve).
    fn external_drive(self) -> bool {
        self != SatMode::Console
    }
}

const TUBE_BIAS: f32 = 0.5;
const TAPE_BIAS: f32 = 0.15;
const TRANSFORMER_BIAS: f32 = 0.1;
/// Tape voicing at 15 ips.
const TAPE_SPEED_IPS: f32 = 15.0;
const TAPE_BUMP_DB: f32 = 2.0;
const TAPE_DIP_DB: f32 = -1.0;
const TAPE_HF_STATIC_DB: f32 = -0.5;
const TAPE_HF_DYNAMIC_DB: f32 = -2.5;
const TAPE_HF_REFERENCE: f32 = 0.25;
/// Transformer voicing: bass reaches the curve 6 dB hotter below 200 Hz.
const TRANSFORMER_EMPHASIS_HZ: f32 = 200.0;
const TRANSFORMER_EMPHASIS_DB: f32 = 6.0;
const TRANSFORMER_SUBSONIC_HZ: f32 = 18.0;
const TRANSFORMER_SUBSONIC_Q: f32 = 0.9;
const TRANSFORMER_HF_HZ: f32 = 16_000.0;
const TRANSFORMER_HF_DB: f32 = 0.7;

/// Console curve drive for a linear stage drive: 0 dB is the identity,
/// +6 dB the Airwindows channel curve (1), capped at the curve's 4.
fn console_drive(drive: f32) -> f32 {
    (drive - 1.0).clamp(0.0, 4.0)
}

/// Output gain that pins a full-scale input near full scale: one over
/// the curve's larger magnitude at `±u`, `u` the driven full-scale
/// input (the drive, or 1 for Console).
pub fn peak_gain(mode: SatMode, drive: f32, curve: f32) -> f32 {
    let Some(c) = mode.curve(drive, curve) else {
        return 1.0;
    };
    let u = if mode.external_drive() { drive as f64 } else { 1.0 };
    let peak = c.eval(u).abs().max(c.eval(-u).abs());
    (1.0 / peak.max(1e-6)) as f32
}

/// One channel of a non-Blend mode.
pub struct ModeChannel {
    wet_os: Oversampler,
    /// The dry path's matching up/down pair.
    dry_os: Oversampler,
    adaa: Adaa1,
    dc: DcBlocker,
    bump: HeadBump,
    hf_loss: HfLoss,
    emphasis: EmphasisPair,
    subsonic: Biquad,
    hf_bell: Biquad,
}

impl ModeChannel {
    pub fn new(sample_rate: f32) -> Self {
        let os = || {
            let mut o = Oversampler::new();
            o.set_factor(OversampleFactor::X4);
            o
        };
        let mut dc = DcBlocker::default();
        dc.set_cutoff(DcBlocker::DEFAULT_CUTOFF_HZ, sample_rate);
        let mut bump = HeadBump::new();
        bump.set(sample_rate, TAPE_SPEED_IPS, TAPE_BUMP_DB, TAPE_DIP_DB);
        let mut hf_loss = HfLoss::new();
        hf_loss.set_corner(sample_rate, hf_loss_corner_hz(TAPE_SPEED_IPS));
        hf_loss.set_times(sample_rate, 5.0, 80.0);
        hf_loss.set_amounts(TAPE_HF_STATIC_DB, TAPE_HF_DYNAMIC_DB, TAPE_HF_REFERENCE);
        let mut emphasis = EmphasisPair::new();
        emphasis.set(sample_rate, TRANSFORMER_EMPHASIS_HZ, TRANSFORMER_EMPHASIS_DB);
        let mut subsonic = Biquad::identity();
        subsonic.set_high_pass(sample_rate, TRANSFORMER_SUBSONIC_HZ, TRANSFORMER_SUBSONIC_Q);
        let mut hf_bell = Biquad::identity();
        hf_bell.set_bell(sample_rate, TRANSFORMER_HF_HZ, 0.8, TRANSFORMER_HF_DB);
        Self {
            wet_os: os(),
            dry_os: os(),
            adaa: Adaa1::new(),
            dc,
            bump,
            hf_loss,
            emphasis,
            subsonic,
            hf_bell,
        }
    }

    pub fn reset(&mut self) {
        self.wet_os.reset();
        self.dry_os.reset();
        self.adaa.reset();
        self.dc.reset();
        self.bump.reset();
        self.hf_loss.reset();
        self.emphasis.reset();
        self.subsonic.reset();
        self.hf_bell.reset();
    }

    /// One sample: `x` in, the dry/wet blend out. `drive` is linear,
    /// `gain` the [`peak_gain`], `mix` the wet share.
    #[inline]
    pub fn process(
        &mut self,
        mode: SatMode,
        curve: &Curve,
        x: f32,
        drive: f32,
        gain: f32,
        mix: f32,
    ) -> f32 {
        let dry_hi = self.dry_os.upsample(x);
        let dry = self.dry_os.downsample(&dry_hi);

        let into = if mode == SatMode::Transformer {
            self.emphasis.pre(x)
        } else {
            x
        };
        let pre = if mode.external_drive() { drive } else { 1.0 };
        let mut hi = self.wet_os.upsample(into * pre);
        for v in &mut hi {
            *v = self.adaa.process(curve, *v);
        }
        let mut wet = self.wet_os.downsample(&hi) * gain;
        match mode {
            SatMode::Tube | SatMode::Warm => wet = self.dc.process(wet),
            SatMode::Tape => {
                wet = self.dc.process(wet);
                wet = self.bump.process(wet);
                wet = self.hf_loss.process(wet);
            }
            SatMode::Transformer => {
                wet = self.emphasis.post(wet);
                wet = self.subsonic.process(wet);
                wet = self.hf_bell.process(wet);
            }
            SatMode::Console | SatMode::Inflator | SatMode::Blend => {}
        }
        dry + (wet - dry) * mix
    }
}
