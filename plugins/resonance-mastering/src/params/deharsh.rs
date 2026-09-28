//! Plugin-facing params for the de-harsh resonance suppressor (between
//! the corrective EQ and the glue compressor).
//!
//! Every value maps 1:1 onto `resonance_dsp::deharsh::SuppressorConfig`;
//! the precise meaning of each is in
//! `docs/design/deharsh-resonance-suppressor.md` §4. The defaults are the
//! suppressor's own, so the two can never disagree.

use std::sync::Arc;

use resonance_dsp::deharsh::{
    ATTACK_MS_RANGE, DEPTH_DB_RANGE, FREQ_HZ_RANGE, RELEASE_MS_RANGE, SELECTIVITY_DB_RANGE,
    SHARPNESS_Q_RANGE,
};
use resonance_dsp::{SuppressorConfig, SuppressorMode};
use resonance_plugin::formatters::{
    s2v_f32_hz, s2v_f32_percentage, v2s_f32_db, v2s_f32_hz, v2s_f32_ms, v2s_f32_percent,
};
use resonance_plugin::*;

pub const PARAM_COUNT: usize = 11;

/// Labels of `dh_mode`, indexed by [`SuppressorMode::index`].
pub const MODE_LABELS: &[&str] = &SuppressorMode::NAMES;

pub struct DeharshParams {
    pub on: BoolParam,
    pub depth: FloatParam,
    pub selectivity: FloatParam,
    pub sharpness: FloatParam,
    pub attack: FloatParam,
    pub release: FloatParam,
    pub low: FloatParam,
    pub high: FloatParam,
    pub mode: IntParam,
    pub mix: FloatParam,
    pub delta: BoolParam,
}

impl DeharshParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.on,
            1 => &self.depth,
            2 => &self.selectivity,
            3 => &self.sharpness,
            4 => &self.attack,
            5 => &self.release,
            6 => &self.low,
            7 => &self.high,
            8 => &self.mode,
            9 => &self.mix,
            10 => &self.delta,
            _ => &self.on,
        }
    }

    pub fn snapshot(&self) -> SuppressorConfig {
        SuppressorConfig {
            enabled: self.on.value(),
            depth_db: self.depth.value(),
            selectivity_db: self.selectivity.value(),
            sharpness_q: self.sharpness.value(),
            attack_ms: self.attack.value(),
            release_ms: self.release.value(),
            low_hz: self.low.value(),
            high_hz: self.high.value(),
            mode: SuppressorMode::from_index(self.mode.value()),
            mix: self.mix.value(),
            delta: self.delta.value(),
        }
    }
}

fn format_q() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v: f32| format!("Q {v:.1}"))
}

fn range((min, max): (f32, f32)) -> FloatRange {
    FloatRange::Linear { min, max }
}

fn skewed((min, max): (f32, f32)) -> FloatRange {
    FloatRange::Skewed {
        min,
        max,
        factor: FloatRange::skew_factor(-2.0),
    }
}

fn hz(id: &'static str, name: &'static str, default: f32) -> FloatParam {
    FloatParam::new(id, name, default, skewed(FREQ_HZ_RANGE))
        .with_unit(" Hz")
        .with_string_to_value(s2v_f32_hz())
        .with_value_to_string(v2s_f32_hz())
}

impl Default for DeharshParams {
    fn default() -> Self {
        let d = SuppressorConfig::default();
        Self {
            on: BoolParam::new("dh_on", "De-harsh On", d.enabled),
            depth: FloatParam::new(
                "dh_depth",
                "De-harsh Depth",
                d.depth_db,
                range(DEPTH_DB_RANGE),
            )
            .with_unit(" dB")
            .with_value_to_string(v2s_f32_db(1)),
            selectivity: FloatParam::new(
                "dh_selectivity",
                "De-harsh Selectivity",
                d.selectivity_db,
                range(SELECTIVITY_DB_RANGE),
            )
            .with_unit(" dB")
            .with_value_to_string(v2s_f32_db(1)),
            sharpness: FloatParam::new(
                "dh_sharpness",
                "De-harsh Sharpness",
                d.sharpness_q,
                range(SHARPNESS_Q_RANGE),
            )
            .with_unit(" Q")
            .with_value_to_string(format_q()),
            attack: FloatParam::new(
                "dh_attack",
                "De-harsh Attack",
                d.attack_ms,
                skewed(ATTACK_MS_RANGE),
            )
            .with_unit(" ms")
            .with_value_to_string(v2s_f32_ms(1)),
            release: FloatParam::new(
                "dh_release",
                "De-harsh Release",
                d.release_ms,
                skewed(RELEASE_MS_RANGE),
            )
            .with_unit(" ms")
            .with_value_to_string(v2s_f32_ms(0)),
            low: hz("dh_low", "De-harsh Low", d.low_hz),
            high: hz("dh_high", "De-harsh High", d.high_hz),
            mode: IntParam::new(
                "dh_mode",
                "De-harsh Mode",
                d.mode.index(),
                IntRange::Linear {
                    min: 0,
                    max: MODE_LABELS.len() as i32 - 1,
                },
            )
            .with_choices(MODE_LABELS),
            mix: FloatParam::new(
                "dh_mix",
                "De-harsh Mix",
                d.mix,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_string_to_value(s2v_f32_percentage())
            .with_value_to_string(v2s_f32_percent(0)),
            delta: BoolParam::new("dh_delta", "De-harsh Delta", d.delta),
        }
    }
}
