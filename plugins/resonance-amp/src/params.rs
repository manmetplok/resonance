/// Plugin parameters: input/output gain, persisted model path, and file selector.

use parking_lot::Mutex;
use resonance_plugin::*;
use std::sync::Arc;

/// Maximum number of files the selector param supports.
pub const MAX_FILE_INDEX: i32 = 999;

pub struct AmpParams {
    /// Persisted model file path, reloaded on plugin init.
    pub model_path: Arc<Mutex<String>>,

    /// File selector index exposed as a DAW parameter.
    /// The host can automate this to switch between .nam files
    /// found in the same directory as the loaded model.
    pub file_select: IntParam,

    /// Shared file list used by both the display closure and the plugin.
    pub file_list: Arc<Mutex<Vec<String>>>,

    pub input_gain: FloatParam,

    pub output_gain: FloatParam,
}

impl Default for AmpParams {
    fn default() -> Self {
        Self {
            model_path: Arc::new(Mutex::new(String::new())),
            file_list: Arc::new(Mutex::new(Vec::new())),
            file_select: IntParam::new(
                "file_select",
                "Model Select",
                0,
                IntRange::Linear {
                    min: 0,
                    max: MAX_FILE_INDEX,
                },
            )
            .hidden(),
            input_gain: FloatParam::new(
                "input_gain",
                "Input Gain",
                1.0,
                FloatRange::Skewed {
                    min: 0.01,
                    max: 4.0,
                    factor: FloatRange::gain_skew_factor(-40.0, 12.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(50.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
            output_gain: FloatParam::new(
                "output_gain",
                "Output Gain",
                0.5,
                FloatRange::Skewed {
                    min: 0.001,
                    max: 4.0,
                    factor: FloatRange::gain_skew_factor(-60.0, 12.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(50.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}
