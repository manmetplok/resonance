/// Plugin parameters: dry/wet mix, output gain, and persisted IR path.

use nih_plug::prelude::*;
use nih_plug_iced::IcedState;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::editor;

#[derive(Params)]
pub struct IrParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<IcedState>,

    #[persist = "ir-path"]
    pub ir_path: Arc<Mutex<String>>,

    #[id = "dry_wet"]
    pub dry_wet: FloatParam,

    #[id = "output_gain"]
    pub output_gain: FloatParam,
}

impl Default for IrParams {
    fn default() -> Self {
        Self {
            editor_state: editor::default_state(),
            ir_path: Arc::new(Mutex::new(String::new())),
            dry_wet: FloatParam::new(
                "Dry/Wet",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(50.0))
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
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
