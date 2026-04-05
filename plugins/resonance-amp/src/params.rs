/// Plugin parameters: input/output gain and persisted model path.

use nih_plug::prelude::*;
use nih_plug_iced::IcedState;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::editor;

#[derive(Params)]
pub struct AmpParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<IcedState>,

    /// Persisted model file path, reloaded on plugin init.
    #[persist = "model-path"]
    pub model_path: Arc<Mutex<String>>,

    #[id = "input_gain"]
    pub input_gain: FloatParam,

    #[id = "output_gain"]
    pub output_gain: FloatParam,
}

impl Default for AmpParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            model_path: Arc::new(Mutex::new(String::new())),
            input_gain: FloatParam::new(
                "Input Gain",
                1.0,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 10.0,
                    factor: FloatRange::gain_skew_factor(-20.0, 20.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(50.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
            output_gain: FloatParam::new(
                "Output Gain",
                1.0,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 10.0,
                    factor: FloatRange::gain_skew_factor(-20.0, 20.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(50.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}
