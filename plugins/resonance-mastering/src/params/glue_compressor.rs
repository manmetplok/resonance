//! Plugin-facing params for the glue compressor stage.

use resonance_plugin::formatters::{v2s_f32_db, v2s_f32_ms, v2s_f32_percent, v2s_f32_ratio};
use resonance_plugin::*;

use crate::stages::glue_compressor::GlueCompressorConfig;

pub const PARAM_COUNT: usize = 8;

pub struct GlueCompressorParams {
    pub on: BoolParam,
    pub threshold: FloatParam,
    pub ratio: FloatParam,
    pub attack: FloatParam,
    pub release: FloatParam,
    pub knee: FloatParam,
    pub makeup: FloatParam,
    pub mix: FloatParam,
}

impl GlueCompressorParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.on,
            1 => &self.threshold,
            2 => &self.ratio,
            3 => &self.attack,
            4 => &self.release,
            5 => &self.knee,
            6 => &self.makeup,
            7 => &self.mix,
            _ => &self.on,
        }
    }

    pub fn snapshot(&self) -> GlueCompressorConfig {
        GlueCompressorConfig {
            enabled: self.on.value(),
            threshold_db: self.threshold.value(),
            ratio: self.ratio.value(),
            attack_ms: self.attack.value(),
            release_ms: self.release.value(),
            knee_db: self.knee.value(),
            makeup_db: self.makeup.value(),
            mix: self.mix.value(),
        }
    }
}

impl Default for GlueCompressorParams {
    fn default() -> Self {
        Self {
            on: BoolParam::new("glue_on", "Glue On", false),
            threshold: FloatParam::new(
                "glue_threshold",
                "Glue Threshold",
                -18.0,
                FloatRange::Linear {
                    min: -40.0,
                    max: 0.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(v2s_f32_db(1)),
            ratio: FloatParam::new(
                "glue_ratio",
                "Glue Ratio",
                2.0,
                FloatRange::Linear { min: 1.0, max: 8.0 },
            )
            .with_value_to_string(v2s_f32_ratio()),
            attack: FloatParam::new(
                "glue_attack",
                "Glue Attack",
                30.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 200.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(v2s_f32_ms(1)),
            release: FloatParam::new(
                "glue_release",
                "Glue Release",
                150.0,
                FloatRange::Skewed {
                    min: 10.0,
                    max: 1000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(v2s_f32_ms(0)),
            knee: FloatParam::new(
                "glue_knee",
                "Glue Knee",
                6.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 12.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(v2s_f32_db(1)),
            makeup: FloatParam::new(
                "glue_makeup",
                "Glue Makeup",
                0.0,
                FloatRange::Linear {
                    min: -6.0,
                    max: 12.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(v2s_f32_db(1)),
            mix: FloatParam::new(
                "glue_mix",
                "Glue Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(v2s_f32_percent(0)),
        }
    }
}
