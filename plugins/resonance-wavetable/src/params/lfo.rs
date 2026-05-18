use resonance_plugin::*;

pub struct LfoParams {
    pub shape: IntParam,
    pub rate: FloatParam,
    pub depth: FloatParam,
    pub retrigger: BoolParam,
}

impl LfoParams {
    pub(super) fn new(
        num: usize,
        default_rate: f32,
        default_depth: f32,
        default_retrigger: bool,
    ) -> Self {
        let sh_id: &'static str = Box::leak(format!("lfo{}_shape", num).into_boxed_str());
        let sh_name: &'static str = Box::leak(format!("LFO {} Shape", num).into_boxed_str());
        let rt_id: &'static str = Box::leak(format!("lfo{}_rate", num).into_boxed_str());
        let rt_name: &'static str = Box::leak(format!("LFO {} Rate", num).into_boxed_str());
        let dp_id: &'static str = Box::leak(format!("lfo{}_depth", num).into_boxed_str());
        let dp_name: &'static str = Box::leak(format!("LFO {} Depth", num).into_boxed_str());
        let rtr_id: &'static str = Box::leak(format!("lfo{}_retrigger", num).into_boxed_str());
        let rtr_name: &'static str = Box::leak(format!("LFO {} Retrigger", num).into_boxed_str());

        Self {
            shape: IntParam::new(sh_id, sh_name, 0, IntRange::Linear { min: 0, max: 4 }),
            rate: FloatParam::new(
                rt_id,
                rt_name,
                default_rate,
                FloatRange::Skewed {
                    min: 0.01,
                    max: 50.0,
                    factor: -2.0,
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            depth: FloatParam::new(
                dp_id,
                dp_name,
                default_depth,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            retrigger: BoolParam::new(rtr_id, rtr_name, default_retrigger),
        }
    }
}
