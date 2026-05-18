use resonance_plugin::*;

pub struct UnisonParams {
    pub voices: IntParam,
    pub detune: FloatParam,
    pub spread: FloatParam,
}

impl UnisonParams {
    pub(super) fn new() -> Self {
        Self {
            voices: IntParam::new(
                "unison_voices",
                "Unison Voices",
                1,
                IntRange::Linear { min: 1, max: 7 },
            ),
            detune: FloatParam::new(
                "unison_detune",
                "Unison Detune",
                15.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 100.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ct")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            spread: FloatParam::new(
                "unison_spread",
                "Unison Spread",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
        }
    }
}
