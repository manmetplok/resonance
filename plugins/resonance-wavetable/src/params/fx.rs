use resonance_plugin::*;
use resonance_dsp::OversampleFactor;

use crate::dsp::effects::{DistMode, TONE_OPEN_HZ};

pub struct ChorusParams {
    pub enabled: BoolParam,
    pub rate: FloatParam,
    pub depth: FloatParam,
    pub mix: FloatParam,
}

pub struct DelayParams {
    pub enabled: BoolParam,
    pub time_l: FloatParam,
    pub time_r: FloatParam,
    pub feedback: FloatParam,
    pub mix: FloatParam,
}

pub struct DistortionParams {
    pub enabled: BoolParam,
    pub drive: FloatParam,
    pub mix: FloatParam,
    /// Waveshaper curve, a [`DistMode`] discriminant. Default `Soft` is the
    /// stage's original `tanh` curve.
    pub mode: IntParam,
    /// Run the master stage at 1×/2×/4× ([`OversampleFactor`]). Default Off.
    pub oversample: IntParam,
    /// Post-shaper one-pole low-pass on the wet signal, in Hz. At its
    /// maximum ([`TONE_OPEN_HZ`], the default) the filter is bypassed
    /// outright rather than run wide open.
    pub tone: FloatParam,
    /// Scale the wet signal down by the level the drive added, so sweeping
    /// drive changes the character more than the loudness.
    pub auto_gain: BoolParam,
    /// `Crush` mode's quantiser depth. Fractional values are allowed, so it
    /// modulates and automates without stepping.
    pub bits: FloatParam,
    /// `Crush` mode's sample-and-hold rate, in Hz.
    pub crush_rate: FloatParam,
    /// Per-voice saturation *before* the filter, 0..1; 0 bypasses it.
    ///
    /// Not part of the master stage and not gated by [`Self::enabled`]: it
    /// lives here only because it is the other half of the synth's drive.
    /// Each voice is saturated on its own, so a chord keeps its notes'
    /// intermodulation to within each voice — the way a polysynth with a
    /// drive per voice card sounds — instead of all of them clipping one
    /// shared bus. It differs from `filter_drive`, which also saturates per
    /// voice before the SVF, in that it runs whether or not the filter is
    /// on, fades in continuously from 0 (the filter's drive switches in a
    /// fixed `tanh` stage the moment it leaves 0), reaches much harder (up
    /// to 12× gain against the filter's 6×), and is a modulation
    /// destination. The two stack: voice drive first, then the filter's.
    pub voice_drive: FloatParam,
}

impl ChorusParams {
    pub(super) fn new() -> Self {
        Self {
            enabled: BoolParam::new("chorus_enabled", "Chorus On", false),
            rate: FloatParam::new(
                "chorus_rate",
                "Chorus Rate",
                1.0,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 5.0,
                    factor: -1.0,
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            depth: FloatParam::new(
                "chorus_depth",
                "Chorus Depth",
                0.3,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            mix: FloatParam::new(
                "chorus_mix",
                "Chorus Mix",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}

impl DelayParams {
    pub(super) fn new() -> Self {
        Self {
            enabled: BoolParam::new("delay_enabled", "Delay On", false),
            time_l: FloatParam::new(
                "delay_time_l",
                "Delay Time L",
                375.0,
                FloatRange::Skewed {
                    min: 10.0,
                    max: 2000.0,
                    factor: -1.5,
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            time_r: FloatParam::new(
                "delay_time_r",
                "Delay Time R",
                500.0,
                FloatRange::Skewed {
                    min: 10.0,
                    max: 2000.0,
                    factor: -1.5,
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            feedback: FloatParam::new(
                "delay_feedback",
                "Delay Feedback",
                0.4,
                FloatRange::Linear {
                    min: 0.0,
                    max: 0.95,
                },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            mix: FloatParam::new(
                "delay_mix",
                "Delay Mix",
                0.25,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}

impl DistortionParams {
    pub(super) fn new() -> Self {
        Self {
            enabled: BoolParam::new("dist_enabled", "Distortion On", false),
            drive: FloatParam::new(
                "dist_drive",
                "Distortion Drive",
                1.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 20.0,
                    factor: -1.5,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            mix: FloatParam::new(
                "dist_mix",
                "Distortion Mix",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            mode: IntParam::new(
                "dist_mode",
                "Distortion Mode",
                DistMode::Soft as i32,
                IntRange::Linear {
                    min: 0,
                    max: (DistMode::LABELS.len() - 1) as i32,
                },
            )
            .with_choices(&DistMode::LABELS),
            oversample: IntParam::new(
                "dist_oversample",
                "Distortion Oversampling",
                OversampleFactor::Off as i32,
                IntRange::Linear {
                    min: 0,
                    max: (OversampleFactor::LABELS.len() - 1) as i32,
                },
            )
            .with_choices(&OversampleFactor::LABELS),
            tone: FloatParam::new(
                "dist_tone",
                "Distortion Tone",
                TONE_OPEN_HZ,
                FloatRange::Skewed {
                    min: 500.0,
                    max: TONE_OPEN_HZ,
                    factor: -2.0,
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            auto_gain: BoolParam::new("dist_auto_gain", "Distortion Auto Gain", false),
            bits: FloatParam::new(
                "dist_bits",
                "Distortion Bits",
                8.0,
                FloatRange::Linear {
                    min: 1.0,
                    max: 16.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            crush_rate: FloatParam::new(
                "dist_crush_rate",
                "Distortion Crush Rate",
                11025.0,
                FloatRange::Skewed {
                    min: 200.0,
                    max: 48000.0,
                    factor: -2.0,
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            voice_drive: FloatParam::new(
                "voice_drive",
                "Voice Drive",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}
