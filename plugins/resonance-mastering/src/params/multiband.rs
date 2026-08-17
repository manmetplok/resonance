//! Plugin-facing params for the multiband compressor stage.

use resonance_plugin::formatters::{
    s2v_f32_percentage, v2s_f32_db, v2s_f32_hz, v2s_f32_ms, v2s_f32_percent, v2s_f32_ratio,
};
use resonance_plugin::*;

use crate::stages::multiband::{BandConfig, MultibandConfig, NUM_BANDS};

/// Params per band: the same seven controls the single-band glue stage
/// exposes (on, threshold, ratio, attack, release, knee, mix) plus the
/// band output gain.
pub const PARAMS_PER_BAND: usize = 8;
/// Global params: on + 3 crossovers.
pub const GLOBAL_PARAMS: usize = 1 + 3;
/// Total param count for this stage.
pub const PARAM_COUNT: usize = GLOBAL_PARAMS + NUM_BANDS * PARAMS_PER_BAND;

/// One band's controls. Names and ranges deliberately mirror
/// [`crate::params::GlueCompressorParams`] — a band *is* a glue
/// compressor, so "Attack" means the same thing and covers the same
/// range in both places, and the only extra control is the band's
/// output gain.
pub struct MultibandBandParams {
    pub on: BoolParam,
    pub threshold: FloatParam,
    pub ratio: FloatParam,
    pub attack: FloatParam,
    pub release: FloatParam,
    pub knee: FloatParam,
    pub mix: FloatParam,
    pub gain: FloatParam,
}

impl MultibandBandParams {
    fn new(index: usize) -> Self {
        let prefix = "mb";
        Self {
            on: BoolParam::new(
                leak(format!("{prefix}_b{index}_on")),
                leak(format!("MB B{} On", index + 1)),
                false,
            ),
            threshold: FloatParam::new(
                leak(format!("{prefix}_b{index}_thresh")),
                leak(format!("MB B{} Threshold", index + 1)),
                -18.0,
                FloatRange::Linear {
                    min: -40.0,
                    max: 0.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(v2s_f32_db(1)),
            ratio: FloatParam::new(
                leak(format!("{prefix}_b{index}_ratio")),
                leak(format!("MB B{} Ratio", index + 1)),
                2.0,
                FloatRange::Linear { min: 1.0, max: 8.0 },
            )
            .with_unit(":1")
            .with_value_to_string(v2s_f32_ratio()),
            attack: FloatParam::new(
                leak(format!("{prefix}_b{index}_attack")),
                leak(format!("MB B{} Attack", index + 1)),
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
                leak(format!("{prefix}_b{index}_release")),
                leak(format!("MB B{} Release", index + 1)),
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
                leak(format!("{prefix}_b{index}_knee")),
                leak(format!("MB B{} Knee", index + 1)),
                6.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 12.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(v2s_f32_db(1)),
            mix: FloatParam::new(
                leak(format!("{prefix}_b{index}_mix")),
                leak(format!("MB B{} Mix", index + 1)),
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_string_to_value(s2v_f32_percentage())
            .with_value_to_string(v2s_f32_percent(0)),
            gain: FloatParam::new(
                leak(format!("{prefix}_b{index}_gain")),
                leak(format!("MB B{} Gain", index + 1)),
                0.0,
                FloatRange::Linear {
                    min: -12.0,
                    max: 12.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(v2s_f32_db(1)),
        }
    }

    fn param_at(&self, sub: usize) -> &dyn Param {
        match sub {
            0 => &self.on,
            1 => &self.threshold,
            2 => &self.ratio,
            3 => &self.attack,
            4 => &self.release,
            5 => &self.knee,
            6 => &self.mix,
            7 => &self.gain,
            _ => &self.on,
        }
    }

    fn snapshot(&self) -> BandConfig {
        BandConfig {
            enabled: self.on.value(),
            threshold_db: self.threshold.value(),
            ratio: self.ratio.value(),
            attack_ms: self.attack.value(),
            release_ms: self.release.value(),
            knee_db: self.knee.value(),
            mix: self.mix.value(),
            gain_db: self.gain.value(),
        }
    }
}

pub struct MultibandParams {
    pub on: BoolParam,
    pub xo1: FloatParam,
    pub xo2: FloatParam,
    pub xo3: FloatParam,
    pub bands: [MultibandBandParams; NUM_BANDS],
}

impl MultibandParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.on,
            1 => &self.xo1,
            2 => &self.xo2,
            3 => &self.xo3,
            i if i < PARAM_COUNT => {
                let band_idx = (i - GLOBAL_PARAMS) / PARAMS_PER_BAND;
                let sub = (i - GLOBAL_PARAMS) % PARAMS_PER_BAND;
                self.bands[band_idx].param_at(sub)
            }
            _ => &self.on,
        }
    }

    pub fn snapshot(&self) -> MultibandConfig {
        let mut xo = [self.xo1.value(), self.xo2.value(), self.xo3.value()];
        // Enforce monotonic ordering (xo1 < xo2 < xo3) so the crossover
        // design is always well-defined.
        if xo[1] <= xo[0] {
            xo[1] = xo[0] + 1.0;
        }
        if xo[2] <= xo[1] {
            xo[2] = xo[1] + 1.0;
        }
        MultibandConfig {
            enabled: self.on.value(),
            crossover_hz: xo,
            bands: [
                self.bands[0].snapshot(),
                self.bands[1].snapshot(),
                self.bands[2].snapshot(),
                self.bands[3].snapshot(),
            ],
        }
    }
}

impl Default for MultibandParams {
    fn default() -> Self {
        Self {
            on: BoolParam::new("mb_on", "Multiband On", false),
            xo1: FloatParam::new(
                "mb_xo1",
                "MB Xover 1",
                120.0,
                FloatRange::Skewed {
                    min: 40.0,
                    max: 400.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(v2s_f32_hz()),
            xo2: FloatParam::new(
                "mb_xo2",
                "MB Xover 2",
                800.0,
                FloatRange::Skewed {
                    min: 250.0,
                    max: 2500.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(v2s_f32_hz()),
            xo3: FloatParam::new(
                "mb_xo3",
                "MB Xover 3",
                4000.0,
                FloatRange::Skewed {
                    min: 1500.0,
                    max: 10_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(v2s_f32_hz()),
            bands: [
                MultibandBandParams::new(0),
                MultibandBandParams::new(1),
                MultibandBandParams::new(2),
                MultibandBandParams::new(3),
            ],
        }
    }
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}
