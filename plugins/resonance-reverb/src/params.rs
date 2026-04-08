/// Plugin parameters for the algorithmic reverb.

use nih_plug::prelude::*;

#[derive(Params)]
pub struct ReverbParams {
    #[id = "predelay"]
    pub predelay: FloatParam,

    #[id = "size"]
    pub size: FloatParam,

    #[id = "decay"]
    pub decay: FloatParam,

    #[id = "damping"]
    pub damping: FloatParam,

    #[id = "diffusion"]
    pub diffusion: FloatParam,

    #[id = "mod_rate"]
    pub mod_rate: FloatParam,

    #[id = "mod_depth"]
    pub mod_depth: FloatParam,

    #[id = "width"]
    pub width: FloatParam,

    #[id = "mix"]
    pub mix: FloatParam,

    #[id = "freeze"]
    pub freeze: BoolParam,
}

impl Default for ReverbParams {
    fn default() -> Self {
        Self {
            predelay: FloatParam::new(
                "Pre-delay",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 250.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            size: FloatParam::new(
                "Size",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(100.0))
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            decay: FloatParam::new(
                "Decay",
                2.0,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 30.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_smoother(SmoothingStyle::Linear(100.0))
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            damping: FloatParam::new(
                "Damping",
                8000.0,
                FloatRange::Skewed {
                    min: 200.0,
                    max: 20000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            diffusion: FloatParam::new(
                "Diffusion",
                0.8,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            mod_rate: FloatParam::new(
                "Mod Rate",
                1.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 5.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),

            mod_depth: FloatParam::new(
                "Mod Depth",
                0.3,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            width: FloatParam::new(
                "Width",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            mix: FloatParam::new(
                "Mix",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),

            freeze: BoolParam::new("Freeze", false),
        }
    }
}
