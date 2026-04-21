//! Parametric-EQ stage params — one instance per EQ (corrective / tonal).
//!
//! Each stage owns [`NUM_BANDS`] [`BandParams`] groups, each exposing
//! the five params that describe one biquad section (on, type, freq,
//! Q, gain). [`EqStageParams::snapshot`] converts the atomics into a
//! plain `[BandConfig; NUM_BANDS]` array ready for the DSP engine.

use resonance_plugin::formatters::{v2s_f32_db, v2s_f32_hz};
use resonance_plugin::*;

use crate::stages::linear_phase_eq::{BandConfig, BandType, NUM_BANDS};

/// Number of params exposed per band (on, type, freq, q, gain).
pub const PARAMS_PER_BAND: usize = 5;
/// Number of params per EQ stage.
pub const PARAMS_PER_STAGE: usize = NUM_BANDS * PARAMS_PER_BAND;

pub struct BandParams {
    pub on: BoolParam,
    pub band_type: IntParam,
    pub freq: FloatParam,
    pub q: FloatParam,
    pub gain: FloatParam,
}

impl BandParams {
    pub fn new(prefix: &'static str, band_index: usize, defaults: BandDefaults) -> Self {
        Self {
            on: BoolParam::new(
                leak_id(format!("{prefix}_b{band_index}_on")),
                leak_name(format!("{} B{} On", prefix.to_uppercase(), band_index + 1)),
                defaults.enabled,
            ),
            band_type: IntParam::new(
                leak_id(format!("{prefix}_b{band_index}_type")),
                leak_name(format!(
                    "{} B{} Type",
                    prefix.to_uppercase(),
                    band_index + 1
                )),
                defaults.band_type as i32,
                IntRange::Linear { min: 0, max: 4 },
            ),
            freq: FloatParam::new(
                leak_id(format!("{prefix}_b{band_index}_freq")),
                leak_name(format!(
                    "{} B{} Freq",
                    prefix.to_uppercase(),
                    band_index + 1
                )),
                defaults.freq_hz,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20_000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(v2s_f32_hz()),
            q: FloatParam::new(
                leak_id(format!("{prefix}_b{band_index}_q")),
                leak_name(format!("{} B{} Q", prefix.to_uppercase(), band_index + 1)),
                defaults.q,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 24.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            ),
            gain: FloatParam::new(
                leak_id(format!("{prefix}_b{band_index}_gain")),
                leak_name(format!(
                    "{} B{} Gain",
                    prefix.to_uppercase(),
                    band_index + 1
                )),
                defaults.gain_db,
                FloatRange::Linear {
                    min: -24.0,
                    max: 24.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(v2s_f32_db(1)),
        }
    }

    pub fn param_at(&self, sub_index: usize) -> &dyn Param {
        match sub_index {
            0 => &self.on,
            1 => &self.band_type,
            2 => &self.freq,
            3 => &self.q,
            4 => &self.gain,
            _ => unreachable!("band param sub-index {sub_index}"),
        }
    }

    pub fn snapshot(&self) -> BandConfig {
        BandConfig {
            enabled: self.on.value(),
            band_type: BandType::from_index(self.band_type.value()),
            freq_hz: self.freq.value(),
            q: self.q.value(),
            gain_db: self.gain.value(),
        }
    }
}

/// Full EQ-stage parameter set.
pub struct EqStageParams {
    pub bands: [BandParams; NUM_BANDS],
}

impl EqStageParams {
    pub fn new(prefix: &'static str, defaults: [BandDefaults; NUM_BANDS]) -> Self {
        // Build each band one at a time with its configured defaults.
        let [d0, d1, d2, d3] = defaults;
        Self {
            bands: [
                BandParams::new(prefix, 0, d0),
                BandParams::new(prefix, 1, d1),
                BandParams::new(prefix, 2, d2),
                BandParams::new(prefix, 3, d3),
            ],
        }
    }

    pub fn param_at(&self, index: usize) -> &dyn Param {
        debug_assert!(index < PARAMS_PER_STAGE);
        let band = index / PARAMS_PER_BAND;
        let sub = index % PARAMS_PER_BAND;
        self.bands[band].param_at(sub)
    }

    pub fn snapshot(&self) -> [BandConfig; NUM_BANDS] {
        [
            self.bands[0].snapshot(),
            self.bands[1].snapshot(),
            self.bands[2].snapshot(),
            self.bands[3].snapshot(),
        ]
    }
}

/// Compile-time defaults for a single band — used to seed sensible
/// starting freqs/Qs without enabling the band by default.
#[derive(Debug, Clone, Copy)]
pub struct BandDefaults {
    pub enabled: bool,
    pub band_type: BandType,
    pub freq_hz: f32,
    pub q: f32,
    pub gain_db: f32,
}

/// Corrective EQ starting configuration: aimed at the common problem
/// areas identified in the research brief (mud 250 Hz, boxiness 500 Hz,
/// harshness 3 kHz) plus a rumble HPF. All bands default to OFF.
pub const CORRECTIVE_DEFAULTS: [BandDefaults; NUM_BANDS] = [
    BandDefaults {
        enabled: false,
        band_type: BandType::HighPass,
        freq_hz: 30.0,
        q: 0.707,
        gain_db: 0.0,
    },
    BandDefaults {
        enabled: false,
        band_type: BandType::Bell,
        freq_hz: 250.0,
        q: 2.0,
        gain_db: -3.0,
    },
    BandDefaults {
        enabled: false,
        band_type: BandType::Bell,
        freq_hz: 500.0,
        q: 2.0,
        gain_db: -2.0,
    },
    BandDefaults {
        enabled: false,
        band_type: BandType::Bell,
        freq_hz: 3000.0,
        q: 3.0,
        gain_db: -2.0,
    },
];

/// Tonal EQ starting configuration: broad musical shaping. Four
/// widely-spaced bells from low-mid to air.
pub const TONAL_DEFAULTS: [BandDefaults; NUM_BANDS] = [
    BandDefaults {
        enabled: false,
        band_type: BandType::LowShelf,
        freq_hz: 100.0,
        q: 0.707,
        gain_db: 0.0,
    },
    BandDefaults {
        enabled: false,
        band_type: BandType::Bell,
        freq_hz: 700.0,
        q: 0.8,
        gain_db: 0.0,
    },
    BandDefaults {
        enabled: false,
        band_type: BandType::Bell,
        freq_hz: 2500.0,
        q: 0.8,
        gain_db: 0.0,
    },
    BandDefaults {
        enabled: false,
        band_type: BandType::HighShelf,
        freq_hz: 10_000.0,
        q: 0.707,
        gain_db: 0.0,
    },
];

// --- Helpers -------------------------------------------------------------

/// The plugin `Param` trait wants `&'static str` for id/name, but we
/// want to build them from runtime `prefix` + `band_index` strings.
/// Leak once at plugin construction — the leak is bounded and tied to
/// the plugin's lifetime in practice.
fn leak_id(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn leak_name(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}
