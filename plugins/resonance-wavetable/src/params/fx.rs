use resonance_plugin::*;

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

impl ChorusParams {
    pub(super) fn new() -> Self {
        Self {
            enabled: BoolParam::new("chorus_enabled", "Chorus On", false),
            rate: FloatParam::new(
                "chorus_rate",
                "Chorus Rate",
                1.0,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 5.0,
                    factor: -1.0,
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            depth: FloatParam::new(
                "chorus_depth",
                "Chorus Depth",
                0.3,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            mix: FloatParam::new(
                "chorus_mix",
                "Chorus Mix",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}

impl DelayParams {
    pub(super) fn new() -> Self {
        Self {
            enabled: BoolParam::new("delay_enabled", "Delay On", false),
            time_l: FloatParam::new(
                "delay_time_l",
                "Delay Time L",
                375.0,
                FloatRange::Skewed {
                    min: 10.0,
                    max: 2000.0,
                    factor: -1.5,
                },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            time_r: FloatParam::new(
                "delay_time_r",
                "Delay Time R",
                500.0,
                FloatRange::Skewed {
                    min: 10.0,
                    max: 2000.0,
                    factor: -1.5,
                },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
            feedback: FloatParam::new(
                "delay_feedback",
                "Delay Feedback",
                0.4,
                FloatRange::Linear {
                    min: 0.0,
                    max: 0.95,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            mix: FloatParam::new(
                "delay_mix",
                "Delay Mix",
                0.25,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}

impl DistortionParams {
    pub(super) fn new() -> Self {
        Self {
            enabled: BoolParam::new("dist_enabled", "Distortion On", false),
            drive: FloatParam::new(
                "dist_drive",
                "Distortion Drive",
                1.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 20.0,
                    factor: -1.5,
                },
            )
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            mix: FloatParam::new(
                "dist_mix",
                "Distortion Mix",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}
