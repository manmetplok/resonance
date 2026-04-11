//! Plugin-facing params for the stereo imager.

use std::sync::Arc;

use resonance_plugin::formatters::v2s_f32_hz;
use resonance_plugin::*;

use crate::stages::imager::ImagerConfig;

pub const PARAM_COUNT: usize = 4;

pub struct ImagerParams {
    pub on: BoolParam,
    pub width: FloatParam,
    pub side_hpf_on: BoolParam,
    pub side_hpf_freq: FloatParam,
}

impl ImagerParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.on,
            1 => &self.width,
            2 => &self.side_hpf_on,
            3 => &self.side_hpf_freq,
            _ => unreachable!("imager param index {index}"),
        }
    }

    pub fn snapshot(&self) -> ImagerConfig {
        ImagerConfig {
            enabled: self.on.value(),
            width: self.width.value(),
            side_hpf_on: self.side_hpf_on.value(),
            side_hpf_hz: self.side_hpf_freq.value(),
        }
    }
}

fn format_width() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v: f32| {
        if v < 0.05 {
            "Mono".to_string()
        } else if (v - 1.0).abs() < 0.02 {
            "Stereo".to_string()
        } else {
            format!("{:.0}%", v * 100.0)
        }
    })
}

impl Default for ImagerParams {
    fn default() -> Self {
        Self {
            on: BoolParam::new("img_on", "Imager On", false),
            width: FloatParam::new(
                "img_width",
                "Width",
                1.0,
                FloatRange::Linear { min: 0.0, max: 2.0 },
            )
            .with_value_to_string(format_width()),
            side_hpf_on: BoolParam::new("img_side_hpf_on", "Side HPF On", false),
            side_hpf_freq: FloatParam::new(
                "img_side_hpf_freq",
                "Side HPF",
                120.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 400.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(v2s_f32_hz()),
        }
    }
}
