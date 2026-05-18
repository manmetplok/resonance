use resonance_plugin::*;

use crate::wavetable::NUM_WAVETABLES;

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
                IntRange::Linear {
                    min: 0,
                    max: (NUM_WAVETABLES - 1) as i32,
                },
            ),
            position: FloatParam::new(
                pos_id,
                pos_name,
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
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
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ct")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            level: FloatParam::new(
                level_id,
                level_name,
                default_level,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(5.0))
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
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            enabled: BoolParam::new(en_id, en_name, default_enabled),
        }
    }
}
