/// All parameters for the wavetable synthesizer (87 total).

use resonance_plugin::*;

use crate::modulation::NUM_MOD_SLOTS;
use crate::wavetable::NUM_WAVETABLES;

// ---------------------------------------------------------------------------
// Sub-structs
// ---------------------------------------------------------------------------

pub struct OscParams {
    pub wavetable: IntParam,
    pub position: FloatParam,
    pub coarse: IntParam,
    pub fine: FloatParam,
    pub level: FloatParam,
    pub pan: FloatParam,
    pub enabled: BoolParam,
}

pub struct EnvParams {
    pub attack: FloatParam,
    pub decay: FloatParam,
    pub sustain: FloatParam,
    pub release: FloatParam,
    pub curve: FloatParam,
}

pub struct LfoParams {
    pub shape: IntParam,
    pub rate: FloatParam,
    pub depth: FloatParam,
    pub retrigger: BoolParam,
}

pub struct FilterParams {
    pub filter_type: IntParam,
    pub cutoff: FloatParam,
    pub resonance: FloatParam,
    pub env_depth: FloatParam,
    pub keytrack: FloatParam,
    pub enabled: BoolParam,
    pub drive: FloatParam,
}

pub struct UnisonParams {
    pub voices: IntParam,
    pub detune: FloatParam,
    pub spread: FloatParam,
}

pub struct ModSlotParams {
    pub source: IntParam,
    pub destination: IntParam,
    pub amount: FloatParam,
}

pub struct ChorusParams {
    pub enabled: BoolParam,
    pub rate: FloatParam,
    pub depth: FloatParam,
    pub mix: FloatParam,
}

pub struct DelayParams {
    pub enabled: BoolParam,
    pub time_l: FloatParam,
    pub time_r: FloatParam,
    pub feedback: FloatParam,
    pub mix: FloatParam,
}

pub struct DistortionParams {
    pub enabled: BoolParam,
    pub drive: FloatParam,
    pub mix: FloatParam,
}

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
                FloatRange::Skewed { min: 0.0, max: 2000.0, factor: -2.0 },
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
                FloatRange::Linear { min: -1.0, max: 1.0 },
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
            mod_slots: (0..NUM_MOD_SLOTS).map(|i| ModSlotParams::new(i)).collect(),

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

// ---------------------------------------------------------------------------
// Sub-struct constructors
// ---------------------------------------------------------------------------

impl OscParams {
    fn new(num: usize, default_wt: i32, default_level: f32, default_enabled: bool) -> Self {
        let wt_id: &'static str = Box::leak(format!("osc{}_wavetable", num).into_boxed_str());
        let wt_name: &'static str = Box::leak(format!("Osc {} Wavetable", num).into_boxed_str());
        let pos_id: &'static str = Box::leak(format!("osc{}_position", num).into_boxed_str());
        let pos_name: &'static str = Box::leak(format!("Osc {} Position", num).into_boxed_str());
        let coarse_id: &'static str = Box::leak(format!("osc{}_coarse", num).into_boxed_str());
        let coarse_name: &'static str = Box::leak(format!("Osc {} Coarse", num).into_boxed_str());
        let fine_id: &'static str = Box::leak(format!("osc{}_fine", num).into_boxed_str());
        let fine_name: &'static str = Box::leak(format!("Osc {} Fine", num).into_boxed_str());
        let level_id: &'static str = Box::leak(format!("osc{}_level", num).into_boxed_str());
        let level_name: &'static str = Box::leak(format!("Osc {} Level", num).into_boxed_str());
        let pan_id: &'static str = Box::leak(format!("osc{}_pan", num).into_boxed_str());
        let pan_name: &'static str = Box::leak(format!("Osc {} Pan", num).into_boxed_str());
        let en_id: &'static str = Box::leak(format!("osc{}_enabled", num).into_boxed_str());
        let en_name: &'static str = Box::leak(format!("Osc {} On", num).into_boxed_str());

        Self {
            wavetable: IntParam::new(
                wt_id,
                wt_name,
                default_wt,
                IntRange::Linear { min: 0, max: (NUM_WAVETABLES - 1) as i32 },
            ),
            position: FloatParam::new(
                pos_id, pos_name, 0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            coarse: IntParam::new(
                coarse_id, coarse_name, 0,
                IntRange::Linear { min: -24, max: 24 },
            ),
            fine: FloatParam::new(
                fine_id, fine_name, 0.0,
                FloatRange::Linear { min: -100.0, max: 100.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ct")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            level: FloatParam::new(
                level_id, level_name, default_level,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            pan: FloatParam::new(
                pan_id, pan_name, 0.0,
                FloatRange::Linear { min: -1.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            enabled: BoolParam::new(en_id, en_name, default_enabled),
        }
    }
}

impl EnvParams {
    fn new(
        prefix: &str,
        label: &str,
        default_attack: f32,
        default_decay: f32,
        default_sustain: f32,
        default_release: f32,
    ) -> Self {
        let a_id: &'static str = Box::leak(format!("{}_attack", prefix).into_boxed_str());
        let a_name: &'static str = Box::leak(format!("{} Attack", label).into_boxed_str());
        let d_id: &'static str = Box::leak(format!("{}_decay", prefix).into_boxed_str());
        let d_name: &'static str = Box::leak(format!("{} Decay", label).into_boxed_str());
        let s_id: &'static str = Box::leak(format!("{}_sustain", prefix).into_boxed_str());
        let s_name: &'static str = Box::leak(format!("{} Sustain", label).into_boxed_str());
        let r_id: &'static str = Box::leak(format!("{}_release", prefix).into_boxed_str());
        let r_name: &'static str = Box::leak(format!("{} Release", label).into_boxed_str());
        let c_id: &'static str = Box::leak(format!("{}_curve", prefix).into_boxed_str());
        let c_name: &'static str = Box::leak(format!("{} Curve", label).into_boxed_str());

        Self {
            attack: FloatParam::new(
                a_id, a_name, default_attack,
                FloatRange::Skewed { min: 0.001, max: 5.0, factor: -2.0 },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(3)),
            decay: FloatParam::new(
                d_id, d_name, default_decay,
                FloatRange::Skewed { min: 0.001, max: 10.0, factor: -2.0 },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(3)),
            sustain: FloatParam::new(
                s_id, s_name, default_sustain,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            release: FloatParam::new(
                r_id, r_name, default_release,
                FloatRange::Skewed { min: 0.001, max: 10.0, factor: -2.0 },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(3)),
            curve: FloatParam::new(
                c_id, c_name, 0.0,
                FloatRange::Linear { min: -1.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
        }
    }
}

impl FilterParams {
    fn new() -> Self {
        Self {
            filter_type: IntParam::new(
                "filter_type", "Filter Type", 0,
                IntRange::Linear { min: 0, max: 3 },
            ),
            cutoff: FloatParam::new(
                "filter_cutoff", "Filter Cutoff", 8000.0,
                FloatRange::Skewed { min: 20.0, max: 20000.0, factor: -2.5 },
            )
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            resonance: FloatParam::new(
                "filter_resonance", "Filter Resonance", 0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            env_depth: FloatParam::new(
                "filter_env_depth", "Filter Env Depth", 0.0,
                FloatRange::Linear { min: -1.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            keytrack: FloatParam::new(
                "filter_keytrack", "Filter Key Track", 0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            enabled: BoolParam::new("filter_enabled", "Filter On", true),
            drive: FloatParam::new(
                "filter_drive", "Filter Drive", 0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}

impl LfoParams {
    fn new(num: usize, default_rate: f32, default_depth: f32, default_retrigger: bool) -> Self {
        let sh_id: &'static str = Box::leak(format!("lfo{}_shape", num).into_boxed_str());
        let sh_name: &'static str = Box::leak(format!("LFO {} Shape", num).into_boxed_str());
        let rt_id: &'static str = Box::leak(format!("lfo{}_rate", num).into_boxed_str());
        let rt_name: &'static str = Box::leak(format!("LFO {} Rate", num).into_boxed_str());
        let dp_id: &'static str = Box::leak(format!("lfo{}_depth", num).into_boxed_str());
        let dp_name: &'static str = Box::leak(format!("LFO {} Depth", num).into_boxed_str());
        let rtr_id: &'static str = Box::leak(format!("lfo{}_retrigger", num).into_boxed_str());
        let rtr_name: &'static str =
            Box::leak(format!("LFO {} Retrigger", num).into_boxed_str());

        Self {
            shape: IntParam::new(sh_id, sh_name, 0, IntRange::Linear { min: 0, max: 4 }),
            rate: FloatParam::new(
                rt_id, rt_name, default_rate,
                FloatRange::Skewed { min: 0.01, max: 50.0, factor: -2.0 },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            depth: FloatParam::new(
                dp_id, dp_name, default_depth,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            retrigger: BoolParam::new(rtr_id, rtr_name, default_retrigger),
        }
    }
}

impl UnisonParams {
    fn new() -> Self {
        Self {
            voices: IntParam::new(
                "unison_voices", "Unison Voices", 1,
                IntRange::Linear { min: 1, max: 7 },
            ),
            detune: FloatParam::new(
                "unison_detune", "Unison Detune", 15.0,
                FloatRange::Linear { min: 0.0, max: 100.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ct")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            spread: FloatParam::new(
                "unison_spread", "Unison Spread", 0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}

impl ModSlotParams {
    fn new(index: usize) -> Self {
        let src_id: &'static str = Box::leak(format!("mod_{}_src", index + 1).into_boxed_str());
        let src_name: &'static str =
            Box::leak(format!("Mod {} Source", index + 1).into_boxed_str());
        let dst_id: &'static str = Box::leak(format!("mod_{}_dst", index + 1).into_boxed_str());
        let dst_name: &'static str =
            Box::leak(format!("Mod {} Dest", index + 1).into_boxed_str());
        let amt_id: &'static str = Box::leak(format!("mod_{}_amt", index + 1).into_boxed_str());
        let amt_name: &'static str =
            Box::leak(format!("Mod {} Amount", index + 1).into_boxed_str());

        Self {
            source: IntParam::new(src_id, src_name, 0, IntRange::Linear { min: 0, max: 8 }),
            destination: IntParam::new(dst_id, dst_name, 0, IntRange::Linear { min: 0, max: 11 }),
            amount: FloatParam::new(
                amt_id, amt_name, 0.0,
                FloatRange::Linear { min: -1.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
        }
    }
}

impl ChorusParams {
    fn new() -> Self {
        Self {
            enabled: BoolParam::new("chorus_enabled", "Chorus On", false),
            rate: FloatParam::new(
                "chorus_rate", "Chorus Rate", 1.0,
                FloatRange::Skewed { min: 0.1, max: 5.0, factor: -1.0 },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            depth: FloatParam::new(
                "chorus_depth", "Chorus Depth", 0.3,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            mix: FloatParam::new(
                "chorus_mix", "Chorus Mix", 0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}

impl DelayParams {
    fn new() -> Self {
        Self {
            enabled: BoolParam::new("delay_enabled", "Delay On", false),
            time_l: FloatParam::new(
                "delay_time_l", "Delay Time L", 375.0,
                FloatRange::Skewed { min: 10.0, max: 2000.0, factor: -1.5 },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            time_r: FloatParam::new(
                "delay_time_r", "Delay Time R", 500.0,
                FloatRange::Skewed { min: 10.0, max: 2000.0, factor: -1.5 },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            feedback: FloatParam::new(
                "delay_feedback", "Delay Feedback", 0.4,
                FloatRange::Linear { min: 0.0, max: 0.95 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            mix: FloatParam::new(
                "delay_mix", "Delay Mix", 0.25,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}

impl DistortionParams {
    fn new() -> Self {
        Self {
            enabled: BoolParam::new("dist_enabled", "Distortion On", false),
            drive: FloatParam::new(
                "dist_drive", "Distortion Drive", 1.0,
                FloatRange::Skewed { min: 1.0, max: 20.0, factor: -1.5 },
            )
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            mix: FloatParam::new(
                "dist_mix", "Distortion Mix", 0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}
