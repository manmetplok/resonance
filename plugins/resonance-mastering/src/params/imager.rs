//! Plugin-facing params for the stereo imager.
//!
//! The four per-band widths (`img_b{n}_width`, warmth-width-depth.md
//! W9) came after the stage's original four params, so they are listed
//! separately ([`ImagerParams::band_width_param_at`]) and appended after
//! every older param. They act on the multiband's crossover bands.

use std::sync::Arc;

use resonance_plugin::formatters::{s2v_f32_percentage, v2s_f32_hz};
use resonance_plugin::*;

use crate::stages::imager::ImagerConfig;
use crate::stages::multiband::NUM_BANDS;

/// The stage's original params (on, width, side HPF on/freq).
pub const PARAM_COUNT: usize = 4;
/// The per-band widths, appended after every pre-W9 param.
pub const BAND_WIDTH_PARAM_COUNT: usize = NUM_BANDS;

pub struct ImagerParams {
    pub on: BoolParam,
    pub width: FloatParam,
    pub side_hpf_on: BoolParam,
    pub side_hpf_freq: FloatParam,
    /// Side gain per multiband crossover band, low band first. 1.0 (the
    /// default) is the identity.
    pub band_width: [FloatParam; NUM_BANDS],
}

impl ImagerParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.on,
            1 => &self.width,
            2 => &self.side_hpf_on,
            3 => &self.side_hpf_freq,
            _ => &self.on,
        }
    }

    /// Per-band width `band` (index `0..BAND_WIDTH_PARAM_COUNT`).
    pub fn band_width_param_at(&self, band: usize) -> &dyn Param {
        debug_assert!(band < BAND_WIDTH_PARAM_COUNT);
        &self.band_width[band]
    }

    /// The per-band widths the multiband should apply: the params while
    /// the imager is on, unity (the identity) while it is off.
    pub fn band_width_targets(&self) -> [f32; NUM_BANDS] {
        if self.on.value() {
            std::array::from_fn(|b| self.band_width[b].value())
        } else {
            [1.0; NUM_BANDS]
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

/// Width reads as a percentage of the source image (100 % = untouched);
/// the two landmark settings are named alongside it. The percentage is
/// always printed — the param declares `%` as its unit, and a value
/// string without it would read "Mono%" in the host.
fn format_width() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v: f32| {
        let pct = format!("{:.0}%", v * 100.0);
        if v < 0.05 {
            format!("{pct} (Mono)")
        } else if (v - 1.0).abs() < 0.02 {
            format!("{pct} (Stereo)")
        } else {
            pct
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
            .with_unit("%")
            .with_string_to_value(s2v_f32_percentage())
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
            band_width: std::array::from_fn(|b| {
                FloatParam::new(
                    BAND_WIDTH_IDS[b],
                    BAND_WIDTH_NAMES[b],
                    1.0,
                    FloatRange::Linear { min: 0.0, max: 2.0 },
                )
                .with_unit("%")
                .with_string_to_value(s2v_f32_percentage())
                .with_value_to_string(format_width())
            }),
        }
    }
}

const BAND_WIDTH_IDS: [&str; NUM_BANDS] =
    ["img_b0_width", "img_b1_width", "img_b2_width", "img_b3_width"];
const BAND_WIDTH_NAMES: [&str; NUM_BANDS] = ["Width Low", "Width Low-Mid", "Width High-Mid", "Width High"];
