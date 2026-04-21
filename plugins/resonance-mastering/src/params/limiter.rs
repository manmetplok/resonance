//! Plugin-facing params for the true-peak limiter.

use std::sync::Arc;

use resonance_plugin::formatters::v2s_f32_ms;
use resonance_plugin::*;

use crate::stages::limiter::LimiterConfig;

pub const PARAM_COUNT: usize = 3;

pub struct LimiterParams {
    pub on: BoolParam,
    pub ceiling: FloatParam,
    pub release: FloatParam,
}

impl LimiterParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.on,
            1 => &self.ceiling,
            2 => &self.release,
            _ => unreachable!("limiter param index {index}"),
        }
    }

    pub fn snapshot(&self) -> LimiterConfig {
        LimiterConfig {
            enabled: self.on.value(),
            ceiling_db: self.ceiling.value(),
            release_ms: self.release.value(),
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
        }
    }
}
