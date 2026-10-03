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
        let a_id: &'static str = super::intern(format!("{}_attack", prefix));
        let a_name: &'static str = super::intern(format!("{} Attack", label));
        let d_id: &'static str = super::intern(format!("{}_decay", prefix));
        let d_name: &'static str = super::intern(format!("{} Decay", label));
        let s_id: &'static str = super::intern(format!("{}_sustain", prefix));
        let s_name: &'static str = super::intern(format!("{} Sustain", label));
        let r_id: &'static str = super::intern(format!("{}_release", prefix));
        let r_name: &'static str = super::intern(format!("{} Release", label));
        let c_id: &'static str = super::intern(format!("{}_curve", prefix));
        let c_name: &'static str = super::intern(format!("{} Curve", label));

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
