//! Top-level plugin parameters.
//!
//! Laid out so the DSP stages own their own parameter sub-structs
//! (corrective EQ, glue compressor, saturator, tonal EQ, …) and the
//! root [`MasteringParams`] just wires them together and exposes a
//! single flat enumeration to the CLAP bridge.
//!
//! Params are laid out in **signal order** so a linear scan through
//! them reads top-to-bottom through the processing chain.

pub mod dither;
pub mod eq_stage;
pub mod glue_compressor;
pub mod imager;
pub mod limiter;
pub mod multiband;
pub mod saturator;

use std::sync::Arc;

use resonance_plugin::formatters::v2s_f32_db;
use resonance_plugin::*;

pub use dither::DitherParams;
pub use eq_stage::{BandParams, EqStageParams, CORRECTIVE_DEFAULTS, TONAL_DEFAULTS, PARAMS_PER_STAGE};
pub use glue_compressor::GlueCompressorParams;
pub use imager::ImagerParams;
pub use limiter::LimiterParams;
pub use multiband::MultibandParams;
pub use saturator::SaturatorParams;

/// Number of global (non-stage) params at the top of the list.
const GLOBAL_PARAM_COUNT: usize = 3;
const GLUE_PARAM_COUNT: usize = glue_compressor::PARAM_COUNT;
const SAT_PARAM_COUNT: usize = saturator::PARAM_COUNT;
const MB_PARAM_COUNT: usize = multiband::PARAM_COUNT;
const IMG_PARAM_COUNT: usize = imager::PARAM_COUNT;
const LIM_PARAM_COUNT: usize = limiter::PARAM_COUNT;
const DITH_PARAM_COUNT: usize = dither::PARAM_COUNT;

// Index ranges in signal order:
//   global → corrective → glue → saturator → tonal → multiband
//          → imager → limiter → dither
const CORRECTIVE_BASE: usize = GLOBAL_PARAM_COUNT;
const GLUE_BASE: usize = CORRECTIVE_BASE + PARAMS_PER_STAGE;
const SAT_BASE: usize = GLUE_BASE + GLUE_PARAM_COUNT;
const TONAL_BASE: usize = SAT_BASE + SAT_PARAM_COUNT;
const MB_BASE: usize = TONAL_BASE + PARAMS_PER_STAGE;
const IMG_BASE: usize = MB_BASE + MB_PARAM_COUNT;
const LIM_BASE: usize = IMG_BASE + IMG_PARAM_COUNT;
const DITH_BASE: usize = LIM_BASE + LIM_PARAM_COUNT;

/// Total plugin param count:
/// 3 + 20 + 8 + 5 + 20 + 20 + 4 + 3 + 3 = 86.
pub const PARAM_COUNT: usize = DITH_BASE + DITH_PARAM_COUNT;

pub struct MasteringParams {
    pub bypass: BoolParam,
    pub target_lufs: FloatParam,
    pub input_trim_db: FloatParam,
    pub corrective_eq: EqStageParams,
    pub glue_compressor: GlueCompressorParams,
    pub saturator: SaturatorParams,
    pub tonal_eq: EqStageParams,
    pub multiband: MultibandParams,
    pub imager: ImagerParams,
    pub limiter: LimiterParams,
    pub dither: DitherParams,
}

impl MasteringParams {
    /// Look up a parameter by its linear index. Out-of-range indices
    /// return [`MasteringParams::bypass`] as a safe fallback rather
    /// than panicking — CLAP hosts can send stale indices during
    /// state restores and we'd rather degrade gracefully than crash
    /// the audio thread.
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.bypass,
            1 => &self.target_lufs,
            2 => &self.input_trim_db,
            i if i < GLUE_BASE => self.corrective_eq.param_at(i - CORRECTIVE_BASE),
            i if i < SAT_BASE => self.glue_compressor.param_at(i - GLUE_BASE),
            i if i < TONAL_BASE => self.saturator.param_at(i - SAT_BASE),
            i if i < MB_BASE => self.tonal_eq.param_at(i - TONAL_BASE),
            i if i < IMG_BASE => self.multiband.param_at(i - MB_BASE),
            i if i < LIM_BASE => self.imager.param_at(i - IMG_BASE),
            i if i < DITH_BASE => self.limiter.param_at(i - LIM_BASE),
            i if i < PARAM_COUNT => self.dither.param_at(i - DITH_BASE),
            _ => &self.bypass,
        }
    }
}

fn format_lufs() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v: f32| format!("{:.1} LUFS", v))
}

impl Default for MasteringParams {
    fn default() -> Self {
        Self {
            bypass: BoolParam::new("bypass", "Bypass", false),
            target_lufs: FloatParam::new(
                "target_lufs",
                "Target LUFS",
                -14.0,
                FloatRange::Linear {
                    min: -24.0,
                    max: -6.0,
                },
            )
            .with_value_to_string(format_lufs()),
            input_trim_db: FloatParam::new(
                "input_trim_db",
                "Input Trim",
                0.0,
                FloatRange::Linear {
                    min: -24.0,
                    max: 24.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_unit(" dB")
            .with_value_to_string(v2s_f32_db(1)),
            corrective_eq: EqStageParams::new("corr", CORRECTIVE_DEFAULTS),
            glue_compressor: GlueCompressorParams::default(),
            saturator: SaturatorParams::default(),
            tonal_eq: EqStageParams::new("tone", TONAL_DEFAULTS),
            multiband: MultibandParams::default(),
            imager: ImagerParams::default(),
            limiter: LimiterParams::default(),
            dither: DitherParams::default(),
        }
    }
}
