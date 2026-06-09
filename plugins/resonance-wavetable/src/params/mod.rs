//! All parameters for the wavetable synthesizer (87 total).
//!
//! The aggregate [`WavetableParams`] struct is intentionally flat — each
//! section (oscillator, envelope, LFO, filter, unison, modulation matrix,
//! FX) lives in its own submodule and is re-exported here so callers can
//! continue to refer to `crate::params::OscParams`, etc.

use resonance_plugin::*;

use crate::dsp::modulation::NUM_MOD_SLOTS;

pub mod env;
pub mod filter;
pub mod fx;
pub mod lfo;
pub mod mod_slot;
pub mod modulation;
pub mod osc;
pub mod unison;

pub use env::EnvParams;
pub use filter::FilterParams;
pub use fx::{ChorusParams, DelayParams, DistortionParams};
pub use lfo::LfoParams;
pub use mod_slot::ModSlotParams;
pub use osc::OscParams;
pub use unison::UnisonParams;

// ---------------------------------------------------------------------------
// Main params struct
// ---------------------------------------------------------------------------

pub struct WavetableParams {
    pub master_volume: FloatParam,
    pub glide_time: FloatParam,
    pub glide_enabled: BoolParam,
    pub max_voices: IntParam,
    pub osc_balance: FloatParam,
    pub osc1: OscParams,
    pub osc2: OscParams,
    pub unison: UnisonParams,
    pub amp_env: EnvParams,
    pub mod_env: EnvParams,
    pub filter: FilterParams,
    pub lfo1: LfoParams,
    pub lfo2: LfoParams,
    pub lfo3: LfoParams,
    pub mod_slots: Vec<ModSlotParams>,
    pub chorus: ChorusParams,
    pub delay: DelayParams,
    pub distortion: DistortionParams,
}

/// Total number of parameters.
pub const PARAM_COUNT: usize = 87;

impl WavetableParams {
    pub fn new() -> Self {
        Self {
            // Global
            master_volume: FloatParam::new(
                "master_volume",
                "Master Volume",
                0.8,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1)),

            glide_time: FloatParam::new(
                "glide_time",
                "Glide Time",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 2000.0,
                    factor: -2.0,
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            glide_enabled: BoolParam::new("glide_enabled", "Glide", false),

            max_voices: IntParam::new(
                "max_voices",
                "Max Voices",
                16,
                IntRange::Linear { min: 1, max: 32 },
            ),

            osc_balance: FloatParam::new(
                "osc_balance",
                "Osc Balance",
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_rounded(2)),

            // Oscillators
            osc1: OscParams::new(1, 0, 1.0, true),
            osc2: OscParams::new(2, 1, 0.5, false),

            // Unison
            unison: UnisonParams::new(),

            // Envelopes
            amp_env: EnvParams::new("amp", "Amp", 0.005, 0.3, 0.8, 0.3),
            mod_env: EnvParams::new("mod", "Mod", 0.01, 0.5, 0.0, 0.5),

            // Filter
            filter: FilterParams::new(),

            // LFOs
            lfo1: LfoParams::new(1, 1.0, 0.5, true),
            lfo2: LfoParams::new(2, 2.0, 0.3, true),
            lfo3: LfoParams::new(3, 0.5, 0.3, false),

            // Modulation matrix
            mod_slots: (0..NUM_MOD_SLOTS).map(ModSlotParams::new).collect(),

            // Effects
            chorus: ChorusParams::new(),
            delay: DelayParams::new(),
            distortion: DistortionParams::new(),
        }
    }

    /// Access parameter by flat index (0..PARAM_COUNT).
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            // Global (0..5)
            0 => &self.master_volume,
            1 => &self.glide_time,
            2 => &self.glide_enabled,
            3 => &self.max_voices,
            4 => &self.osc_balance,
            // Osc1 (5..12)
            5 => &self.osc1.wavetable,
            6 => &self.osc1.position,
            7 => &self.osc1.coarse,
            8 => &self.osc1.fine,
            9 => &self.osc1.level,
            10 => &self.osc1.pan,
            11 => &self.osc1.enabled,
            // Osc2 (12..19)
            12 => &self.osc2.wavetable,
            13 => &self.osc2.position,
            14 => &self.osc2.coarse,
            15 => &self.osc2.fine,
            16 => &self.osc2.level,
            17 => &self.osc2.pan,
            18 => &self.osc2.enabled,
            // Unison (19..22)
            19 => &self.unison.voices,
            20 => &self.unison.detune,
            21 => &self.unison.spread,
            // Amp Env (22..27)
            22 => &self.amp_env.attack,
            23 => &self.amp_env.decay,
            24 => &self.amp_env.sustain,
            25 => &self.amp_env.release,
            26 => &self.amp_env.curve,
            // Mod Env (27..32)
            27 => &self.mod_env.attack,
            28 => &self.mod_env.decay,
            29 => &self.mod_env.sustain,
            30 => &self.mod_env.release,
            31 => &self.mod_env.curve,
            // Filter (32..39)
            32 => &self.filter.filter_type,
            33 => &self.filter.cutoff,
            34 => &self.filter.resonance,
            35 => &self.filter.env_depth,
            36 => &self.filter.keytrack,
            37 => &self.filter.enabled,
            38 => &self.filter.drive,
            // LFO1 (39..43)
            39 => &self.lfo1.shape,
            40 => &self.lfo1.rate,
            41 => &self.lfo1.depth,
            42 => &self.lfo1.retrigger,
            // LFO2 (43..47)
            43 => &self.lfo2.shape,
            44 => &self.lfo2.rate,
            45 => &self.lfo2.depth,
            46 => &self.lfo2.retrigger,
            // LFO3 (47..51)
            47 => &self.lfo3.shape,
            48 => &self.lfo3.rate,
            49 => &self.lfo3.depth,
            50 => &self.lfo3.retrigger,
            // Mod Matrix (51..75) -- 8 slots x 3
            51..=74 => {
                let slot_offset = index - 51;
                let slot_idx = slot_offset / 3;
                let field = slot_offset % 3;
                match field {
                    0 => &self.mod_slots[slot_idx].source,
                    1 => &self.mod_slots[slot_idx].destination,
                    _ => &self.mod_slots[slot_idx].amount,
                }
            }
            // Chorus (75..79)
            75 => &self.chorus.enabled,
            76 => &self.chorus.rate,
            77 => &self.chorus.depth,
            78 => &self.chorus.mix,
            // Delay (79..84)
            79 => &self.delay.enabled,
            80 => &self.delay.time_l,
            81 => &self.delay.time_r,
            82 => &self.delay.feedback,
            83 => &self.delay.mix,
            // Distortion (84..87)
            84 => &self.distortion.enabled,
            85 => &self.distortion.drive,
            86 => &self.distortion.mix,
            _ => &self.master_volume, // fallback
        }
    }
}

impl Default for WavetableParams {
    fn default() -> Self {
        Self::new()
    }
}
