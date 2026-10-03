use resonance_plugin::*;

use crate::dsp::wavetable::USER_WAVETABLE_INDEX;

pub struct OscParams {
    pub wavetable: IntParam,
    pub position: FloatParam,
    pub coarse: IntParam,
    pub fine: FloatParam,
    pub level: FloatParam,
    pub pan: FloatParam,
    pub enabled: BoolParam,
}

impl OscParams {
    pub(super) fn new(
        num: usize,
        default_wt: i32,
        default_level: f32,
        default_enabled: bool,
    ) -> Self {
        let wt_id: &'static str = super::intern(format!("osc{}_wavetable", num));
        let wt_name: &'static str = super::intern(format!("Osc {} Wavetable", num));
        let pos_id: &'static str = super::intern(format!("osc{}_position", num));
        let pos_name: &'static str = super::intern(format!("Osc {} Position", num));
        let coarse_id: &'static str = super::intern(format!("osc{}_coarse", num));
        let coarse_name: &'static str = super::intern(format!("Osc {} Coarse", num));
        let fine_id: &'static str = super::intern(format!("osc{}_fine", num));
        let fine_name: &'static str = super::intern(format!("Osc {} Fine", num));
        let level_id: &'static str = super::intern(format!("osc{}_level", num));
        let level_name: &'static str = super::intern(format!("Osc {} Level", num));
        let pan_id: &'static str = super::intern(format!("osc{}_pan", num));
        let pan_name: &'static str = super::intern(format!("Osc {} Pan", num));
        let en_id: &'static str = super::intern(format!("osc{}_enabled", num));
        let en_name: &'static str = super::intern(format!("Osc {} On", num));

        Self {
            wavetable: IntParam::new(
                wt_id,
                wt_name,
                default_wt,
                // The bundled tables, then the oscillator's user table one
                // past them — appended so every bundled index (and every
                // preset naming one) keeps its meaning.
                IntRange::Linear {
                    min: 0,
                    max: USER_WAVETABLE_INDEX as i32,
                },
            ),
            position: FloatParam::new(
                pos_id,
                pos_name,
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            coarse: IntParam::new(
                coarse_id,
                coarse_name,
                0,
                IntRange::Linear { min: -24, max: 24 },
            ),
            fine: FloatParam::new(
                fine_id,
                fine_name,
                0.0,
                FloatRange::Linear {
                    min: -100.0,
                    max: 100.0,
                },
            )
            .with_unit(" ct")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            level: FloatParam::new(
                level_id,
                level_name,
                default_level,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            pan: FloatParam::new(
                pan_id,
                pan_name,
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            enabled: BoolParam::new(en_id, en_name, default_enabled),
        }
    }
}
