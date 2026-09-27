//! All parameters for the wavetable synthesizer — [`PARAM_COUNT`] of them.
//!
//! The count lives in exactly one place on purpose: a caller that spells
//! it out as a literal silently truncates when a parameter is added, and
//! `param_at`'s own round-trip test cannot see that (ba todo #1324
//! review).
//!
//! The aggregate [`WavetableParams`] struct is intentionally flat — each
//! section (oscillator, envelope, LFO, filter, unison, modulation matrix,
//! FX) lives in its own submodule and is re-exported here so callers can
//! continue to refer to `crate::params::OscParams`, etc.

use resonance_plugin::*;

use crate::dsp::modulation::NUM_MOD_SLOTS;

pub mod analog;
pub mod env;
pub mod filter;
pub mod fx;
pub mod lfo;
pub mod mod_slot;
pub mod modulation;
pub mod osc;
pub mod sample_hold;
pub mod unison;

pub use analog::AnalogParams;
pub use env::EnvParams;
pub use filter::FilterParams;
pub use fx::{ChorusParams, DelayParams, DistortionParams};
pub use lfo::LfoParams;
pub use mod_slot::ModSlotParams;
pub use osc::OscParams;
pub use sample_hold::SampleHoldParams;
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
    pub analog: AnalogParams,
    pub amp_env: EnvParams,
    pub mod_env: EnvParams,
    pub filter: FilterParams,
    pub lfo1: LfoParams,
    pub lfo2: LfoParams,
    pub lfo3: LfoParams,
    pub mod_slots: Vec<ModSlotParams>,
    pub mod_sh: SampleHoldParams,
    pub chorus: ChorusParams,
    pub delay: DelayParams,
    pub distortion: DistortionParams,
}

/// Total number of parameters.
///
/// 87 until ba todo #1324 added `lfoN_sync` + `lfoN_division` to each of the
/// three LFOs; 97 once the BBD chorus (`chorus_mode`, `chorus_noise`) and
/// analog instability (`osc_phase_random`, `analog`) were added; 104 since
/// the distortion character work added `dist_mode`, `dist_oversample`,
/// `dist_tone`, `dist_auto_gain`, `dist_bits`, `dist_crush_rate` and
/// `voice_drive`. Parameters are addressed by string id everywhere that
/// persists (presets, saved state, CLAP param ids are hashed from the id),
/// so inserting into the middle of [`WavetableParams::param_at`] only
/// changes the order a host lists them in.
///
/// 104 -> 106 when the filter models landed: `filter_model` and
/// `filter_fm`, appended at the end of the table (104, 105) rather than
/// beside the other filter params so the indices of everything already
/// listed stay put.
///
/// 106 -> 110 for the new-mod-sources work's `ModSource::SampleHold`
/// generator (`mod_sh_rate`, `mod_sh_slew`, `mod_sh_sync`,
/// `mod_sh_division`), appended the same way at the very end (106..110).
pub const PARAM_COUNT: usize = 110;

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
            .with_value_to_string(formatters::v2s_f32_rounded(2)),

            // Oscillators
            osc1: OscParams::new(1, 0, 1.0, true),
            osc2: OscParams::new(2, 1, 0.5, false),

            // Unison
            unison: UnisonParams::new(),

            // Analog instability
            analog: AnalogParams::new(),

            // Envelopes
            amp_env: EnvParams::new("amp", "Amp", 0.005, 0.3, 0.8, 0.3),
            mod_env: EnvParams::new("mod", "Mod", 0.01, 0.5, 0.0, 0.5),

            // Filter
            filter: FilterParams::new(),

            // LFOs. Depth defaults to zero on all three, matching
            // `presets/init.json` (ba todo #1354) — a fresh instance must
            // not arrive with three LFOs already modulating. The rates are
            // the resting values a user then dials depth into.
            lfo1: LfoParams::new(1, 1.0, 0.0, true),
            lfo2: LfoParams::new(2, 2.0, 0.0, true),
            lfo3: LfoParams::new(3, 0.5, 0.0, false),

            // Modulation matrix
            mod_slots: (0..NUM_MOD_SLOTS).map(ModSlotParams::new).collect(),
            mod_sh: SampleHoldParams::new(),

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
            // LFO1 (39..45)
            39 => &self.lfo1.shape,
            40 => &self.lfo1.rate,
            41 => &self.lfo1.depth,
            42 => &self.lfo1.retrigger,
            43 => &self.lfo1.sync,
            44 => &self.lfo1.division,
            // LFO2 (45..51)
            45 => &self.lfo2.shape,
            46 => &self.lfo2.rate,
            47 => &self.lfo2.depth,
            48 => &self.lfo2.retrigger,
            49 => &self.lfo2.sync,
            50 => &self.lfo2.division,
            // LFO3 (51..57)
            51 => &self.lfo3.shape,
            52 => &self.lfo3.rate,
            53 => &self.lfo3.depth,
            54 => &self.lfo3.retrigger,
            55 => &self.lfo3.sync,
            56 => &self.lfo3.division,
            // Mod Matrix (57..81) -- 8 slots x 3
            57..=80 => {
                let slot_offset = index - 57;
                let slot_idx = slot_offset / 3;
                let field = slot_offset % 3;
                match field {
                    0 => &self.mod_slots[slot_idx].source,
                    1 => &self.mod_slots[slot_idx].destination,
                    _ => &self.mod_slots[slot_idx].amount,
                }
            }
            // Chorus (81..87)
            81 => &self.chorus.enabled,
            82 => &self.chorus.rate,
            83 => &self.chorus.depth,
            84 => &self.chorus.mix,
            85 => &self.chorus.mode,
            86 => &self.chorus.noise,
            // Delay (87..92)
            87 => &self.delay.enabled,
            88 => &self.delay.time_l,
            89 => &self.delay.time_r,
            90 => &self.delay.feedback,
            91 => &self.delay.mix,
            // Distortion (92..95)
            92 => &self.distortion.enabled,
            93 => &self.distortion.drive,
            94 => &self.distortion.mix,
            // Analog (95..97)
            95 => &self.analog.phase_random,
            96 => &self.analog.drift,
            // Distortion character (97..104)
            97 => &self.distortion.mode,
            98 => &self.distortion.oversample,
            99 => &self.distortion.tone,
            100 => &self.distortion.auto_gain,
            101 => &self.distortion.bits,
            102 => &self.distortion.crush_rate,
            103 => &self.distortion.voice_drive,
            // Filter models (104..106)
            104 => &self.filter.model,
            105 => &self.filter.fm,
            // Sample & Hold source (106..110)
            106 => &self.mod_sh.rate,
            107 => &self.mod_sh.slew,
            108 => &self.mod_sh.sync,
            109 => &self.mod_sh.division,
            _ => &self.master_volume, // fallback
        }
    }
}

impl Default for WavetableParams {
    fn default() -> Self {
        Self::new()
    }
}
