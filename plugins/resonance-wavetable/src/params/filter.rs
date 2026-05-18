use resonance_plugin::*;

pub struct FilterParams {
    pub filter_type: IntParam,
    pub cutoff: FloatParam,
    pub resonance: FloatParam,
    pub env_depth: FloatParam,
    pub keytrack: FloatParam,
    pub enabled: BoolParam,
    pub drive: FloatParam,
}

impl FilterParams {
    pub(super) fn new() -> Self {
        Self {
            filter_type: IntParam::new(
                "filter_type",
                "Filter Type",
                0,
                IntRange::Linear { min: 0, max: 3 },
            ),
            cutoff: FloatParam::new(
                "filter_cutoff",
                "Filter Cutoff",
                8000.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20000.0,
                    factor: -2.5,
                },
            )
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            resonance: FloatParam::new(
                "filter_resonance",
                "Filter Resonance",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            env_depth: FloatParam::new(
                "filter_env_depth",
                "Filter Env Depth",
                0.0,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            keytrack: FloatParam::new(
                "filter_keytrack",
                "Filter Key Track",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            enabled: BoolParam::new("filter_enabled", "Filter On", true),
            drive: FloatParam::new(
                "filter_drive",
                "Filter Drive",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}
