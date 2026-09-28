//! Plugin-facing params for the clipper (between imager and limiter).

use std::sync::Arc;

use resonance_plugin::formatters::{s2v_f32_percentage, v2s_f32_db};
use resonance_plugin::*;

use crate::stages::clipper::ClipperConfig;

pub const PARAM_COUNT: usize = 3;

pub struct ClipperParams {
    pub on: BoolParam,
    pub drive: FloatParam,
    /// 0 = hard, 1 = soft.
    pub shape: FloatParam,
}

impl ClipperParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.on,
            1 => &self.drive,
            2 => &self.shape,
            _ => &self.on,
        }
    }

    pub fn snapshot(&self) -> ClipperConfig {
        ClipperConfig {
            enabled: self.on.value(),
            drive_db: self.drive.value(),
            softness: self.shape.value(),
        }
    }
}

/// Shape reads as its softness percentage, naming the two ends. The
/// percentage is always printed, since the param declares `%`.
fn format_shape() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v: f32| {
        let pct = format!("{:.0}%", v * 100.0);
        if v < 0.05 {
            format!("{pct} (Hard)")
        } else if v > 0.95 {
            format!("{pct} (Soft)")
        } else {
            pct
        }
    })
}

impl Default for ClipperParams {
    fn default() -> Self {
        let d = ClipperConfig::default();
        Self {
            on: BoolParam::new("clip_on", "Clipper On", d.enabled),
            drive: FloatParam::new(
                "clip_drive",
                "Clip Drive",
                d.drive_db,
                FloatRange::Linear {
                    min: 0.0,
                    max: 12.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(v2s_f32_db(1)),
            shape: FloatParam::new(
                "clip_shape",
                "Clip Shape",
                d.softness,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_string_to_value(s2v_f32_percentage())
            .with_value_to_string(format_shape()),
        }
    }
}
