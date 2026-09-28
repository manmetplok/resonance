//! Plugin-facing params for the saturator stage.
//!
//! `sat_mode` and `sat_curve` (warmth-width-depth.md W9) came after the
//! stage's original five params, so they are listed separately
//! ([`SaturatorParams::mode_param_at`]) and appended after every older
//! param. `sat_mode`'s default, Blend, is the original stage.

use std::sync::Arc;

use resonance_plugin::formatters::{s2v_f32_percentage, v2s_f32_db, v2s_f32_percent};
use resonance_plugin::*;

use crate::stages::saturator::{SatMode, SaturatorConfig, Shaper};

/// The stage's original params (on, drive, character, mix, shaper).
pub const PARAM_COUNT: usize = 5;
/// `sat_mode` and `sat_curve`, appended after every pre-W9 param.
pub const MODE_PARAM_COUNT: usize = 2;

pub struct SaturatorParams {
    pub on: BoolParam,
    pub drive: FloatParam,
    pub character: FloatParam,
    pub mix: FloatParam,
    pub shaper: IntParam,
    /// Blend (the original Tube↔Tape blend), Tube, Tape, Transformer,
    /// Console, Warm or Inflator.
    pub mode: IntParam,
    /// The Inflator mode's Curve control, −50 %..+50 %.
    pub curve: FloatParam,
}

impl SaturatorParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.on,
            1 => &self.drive,
            2 => &self.character,
            3 => &self.mix,
            4 => &self.shaper,
            _ => &self.on,
        }
    }

    /// `sat_mode` (0) or `sat_curve` (1).
    pub fn mode_param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.mode,
            _ => &self.curve,
        }
    }

    pub fn snapshot(&self) -> SaturatorConfig {
        SaturatorConfig {
            enabled: self.on.value(),
            drive_db: self.drive.value(),
            character: self.character.value(),
            mix: self.mix.value(),
            shaper: Shaper::from_index(self.shaper.value()),
            mode: SatMode::from_index(self.mode.value()),
            curve: self.curve.value(),
        }
    }
}

/// Character is a blend, so the readout is the blend percentage; at the
/// two extremes it names the model the blend has landed on. The
/// percentage is always printed — the param declares `%` as its unit,
/// and a value string that dropped it would read "Tube%" in the host.
fn format_character() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v: f32| {
        let pct = format!("{:.0}%", v * 100.0);
        if v < 0.15 {
            format!("{pct} (Tube)")
        } else if v > 0.85 {
            format!("{pct} (Tape)")
        } else {
            pct
        }
    })
}

/// The Inflator curve, signed, as the JSFX prints it (−50 %..+50 %).
fn format_curve() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v: f32| format!("{:+.0}%", v * 100.0))
}

impl Default for SaturatorParams {
    fn default() -> Self {
        Self {
            on: BoolParam::new("sat_on", "Sat On", false),
            drive: FloatParam::new(
                "sat_drive",
                "Sat Drive",
                3.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 18.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(v2s_f32_db(1)),
            character: FloatParam::new(
                "sat_character",
                "Sat Character",
                0.3,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_string_to_value(s2v_f32_percentage())
            .with_value_to_string(format_character()),
            mix: FloatParam::new(
                "sat_mix",
                "Sat Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            // 0..1 param displayed as 0..100 %, so parsing needs the
            // matching /100 or "50%" would come back as 50.0.
            .with_string_to_value(s2v_f32_percentage())
            .with_value_to_string(v2s_f32_percent(0)),
            shaper: IntParam::new(
                "sat_shaper",
                "Sat Shaper",
                Shaper::Smooth.to_index(),
                IntRange::Linear { min: 0, max: 1 },
            ),
            mode: IntParam::new(
                "sat_mode",
                "Sat Mode",
                SatMode::Blend.to_index(),
                IntRange::Linear {
                    min: 0,
                    max: SatMode::LABELS.len() as i32 - 1,
                },
            )
            .with_choices(SatMode::LABELS),
            curve: FloatParam::new(
                "sat_curve",
                "Sat Curve",
                0.0,
                FloatRange::Linear {
                    min: -0.5,
                    max: 0.5,
                },
            )
            .with_unit("%")
            .with_string_to_value(s2v_f32_percentage())
            .with_value_to_string(format_curve()),
        }
    }
}
