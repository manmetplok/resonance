//! Oscillator character: the interaction between the two oscillators, a
//! phase warp per oscillator, and the sub and noise sources.
//!
//! Every default is the inert setting — `Sum`, warp `Off` at zero, both
//! levels at zero — so a patch saved before these existed, and a fresh
//! instance, render bit-identically to the synth without them.

use resonance_plugin::*;

use crate::dsp::osc_mix::OscMixMode;
use crate::dsp::sub_noise::{NoiseType, SubOctave, SubWave};
use crate::dsp::warp::WarpMode;

pub struct OscMixParams {
    /// [`OscMixMode`] discriminant.
    pub mode: IntParam,
    /// PM index, ring depth or sync sweep, per mode.
    pub amount: FloatParam,
}

impl OscMixParams {
    pub(super) fn new() -> Self {
        Self {
            mode: IntParam::new(
                "osc_mix_mode",
                "Osc Mix Mode",
                OscMixMode::Sum as i32,
                IntRange::Linear {
                    min: 0,
                    max: (OscMixMode::LABELS.len() - 1) as i32,
                },
            )
            .with_choices(&OscMixMode::LABELS),
            amount: FloatParam::new(
                "osc_mod_amount",
                "Osc Mod Amount",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}

pub struct WarpParams {
    /// [`WarpMode`] discriminant.
    pub mode: IntParam,
    /// -1..=1. Bend uses the sign; the other modes use the magnitude.
    pub amount: FloatParam,
}

impl WarpParams {
    pub(super) fn new(num: usize) -> Self {
        let mode_id: &'static str = super::intern(format!("osc{}_warp_mode", num));
        let mode_name: &'static str =
            super::intern(format!("Osc {} Warp Mode", num));
        let amt_id: &'static str = super::intern(format!("osc{}_warp_amount", num));
        let amt_name: &'static str = super::intern(format!("Osc {} Warp", num));

        Self {
            mode: IntParam::new(
                mode_id,
                mode_name,
                WarpMode::Off as i32,
                IntRange::Linear {
                    min: 0,
                    max: (WarpMode::LABELS.len() - 1) as i32,
                },
            )
            .with_choices(&WarpMode::LABELS),
            amount: FloatParam::new(
                amt_id,
                amt_name,
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
        }
    }
}

pub struct SubParams {
    /// [`SubWave`] discriminant.
    pub waveform: IntParam,
    /// [`SubOctave`] discriminant.
    pub octave: IntParam,
    pub level: FloatParam,
}

impl SubParams {
    pub(super) fn new() -> Self {
        Self {
            waveform: IntParam::new(
                "sub_waveform",
                "Sub Waveform",
                SubWave::Sine as i32,
                IntRange::Linear {
                    min: 0,
                    max: (SubWave::LABELS.len() - 1) as i32,
                },
            )
            .with_choices(&SubWave::LABELS),
            octave: IntParam::new(
                "sub_octave",
                "Sub Octave",
                SubOctave::Down1 as i32,
                IntRange::Linear {
                    min: 0,
                    max: (SubOctave::LABELS.len() - 1) as i32,
                },
            )
            .with_choices(&SubOctave::LABELS),
            level: FloatParam::new(
                "sub_level",
                "Sub Level",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}

pub struct NoiseParams {
    /// [`NoiseType`] discriminant.
    pub noise_type: IntParam,
    pub level: FloatParam,
    /// Spectral tilt, -1 (dark) ..= +1 (bright); 0 leaves the noise as is.
    pub color: FloatParam,
}

impl NoiseParams {
    pub(super) fn new() -> Self {
        Self {
            noise_type: IntParam::new(
                "noise_type",
                "Noise Type",
                NoiseType::White as i32,
                IntRange::Linear {
                    min: 0,
                    max: (NoiseType::LABELS.len() - 1) as i32,
                },
            )
            .with_choices(&NoiseType::LABELS),
            level: FloatParam::new(
                "noise_level",
                "Noise Level",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            color: FloatParam::new(
                "noise_color",
                "Noise Color",
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
        }
    }
}
