use resonance_plugin::*;

pub struct EnvParams {
    pub attack: FloatParam,
    pub decay: FloatParam,
    pub sustain: FloatParam,
    pub release: FloatParam,
    pub curve: FloatParam,
}

impl EnvParams {
    pub(super) fn new(
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
                a_id,
                a_name,
                default_attack,
                FloatRange::Skewed {
                    min: 0.001,
                    max: 5.0,
                    factor: -2.0,
                },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(3)),
            decay: FloatParam::new(
                d_id,
                d_name,
                default_decay,
                FloatRange::Skewed {
                    min: 0.001,
                    max: 10.0,
                    factor: -2.0,
                },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(3)),
            sustain: FloatParam::new(
                s_id,
                s_name,
                default_sustain,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            release: FloatParam::new(
                r_id,
                r_name,
                default_release,
                FloatRange::Skewed {
                    min: 0.001,
                    max: 10.0,
                    factor: -2.0,
                },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(3)),
            curve: FloatParam::new(
                c_id,
                c_name,
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
        }
    }
}
