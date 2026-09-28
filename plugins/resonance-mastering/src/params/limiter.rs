//! Plugin-facing params for the true-peak limiter.

use std::sync::Arc;

use resonance_plugin::formatters::{v2s_f32_db, v2s_f32_ms};
use resonance_plugin::*;

use crate::stages::limiter::{LimiterConfig, MAX_INPUT_GAIN_DB};

/// The limiter's params in the pre-W9 block (on, ceiling, release).
pub const PARAM_COUNT: usize = 3;

/// `lim_gain`, appended after the de-harsh params.
pub const GAIN_PARAM_COUNT: usize = 1;

pub struct LimiterParams {
    pub on: BoolParam,
    pub ceiling: FloatParam,
    pub release: FloatParam,
    /// Input drive into the limiter, dB (see [`LimiterConfig::input_gain_db`]).
    pub gain: FloatParam,
}

impl LimiterParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.on,
            1 => &self.ceiling,
            2 => &self.release,
            _ => &self.on,
        }
    }

    /// The appended `lim_gain` param.
    pub fn gain_param_at(&self, _index: usize) -> &dyn Param {
        &self.gain
    }

    pub fn snapshot(&self) -> LimiterConfig {
        LimiterConfig {
            enabled: self.on.value(),
            ceiling_db: self.ceiling.value(),
            release_ms: self.release.value(),
            input_gain_db: self.gain.value(),
        }
    }
}

/// Limiter ceiling formatter is plugin-local because it uses the
/// `dBTP` unit rather than plain `dB`.
fn format_dbtp(decimals: usize) -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(move |v: f32| format!("{:.*} dBTP", decimals, v))
}

impl Default for LimiterParams {
    fn default() -> Self {
        Self {
            on: BoolParam::new("lim_on", "Limiter On", false),
            ceiling: FloatParam::new(
                "lim_ceiling",
                "Ceiling",
                -0.3,
                FloatRange::Linear {
                    min: -6.0,
                    max: 0.0,
                },
            )
            .with_unit(" dBTP")
            .with_value_to_string(format_dbtp(1)),
            release: FloatParam::new(
                "lim_release",
                "Release",
                50.0,
                FloatRange::Skewed {
                    min: 5.0,
                    max: 500.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(v2s_f32_ms(0)),
            gain: FloatParam::new(
                "lim_gain",
                "Limiter Gain",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: MAX_INPUT_GAIN_DB,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(v2s_f32_db(1)),
        }
    }
}
