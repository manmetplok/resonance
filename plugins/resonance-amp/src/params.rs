/// Plugin parameters: input/output gain, persisted model path, and file selector.

use nih_plug::prelude::*;
use parking_lot::Mutex;
use std::sync::Arc;

/// Maximum number of files the selector param supports.
pub const MAX_FILE_INDEX: i32 = 999;

#[derive(Params)]
pub struct AmpParams {
    /// Persisted model file path, reloaded on plugin init.
    #[persist = "model-path"]
    pub model_path: Arc<Mutex<String>>,

    /// File selector index exposed as a DAW parameter.
    /// The host can automate this to switch between .nam files
    /// found in the same directory as the loaded model.
    #[id = "file_select"]
    pub file_select: IntParam,

    #[id = "input_gain"]
    pub input_gain: FloatParam,

    #[id = "output_gain"]
    pub output_gain: FloatParam,
}

impl Default for AmpParams {
    fn default() -> Self {
        let file_list: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let file_list_display = file_list.clone();

        Self {
            model_path: Arc::new(Mutex::new(String::new())),
            file_select: IntParam::new(
                "Model Select",
                0,
                IntRange::Linear {
                    min: 0,
                    max: MAX_FILE_INDEX,
                },
            )
            .with_value_to_string(Arc::new(move |value| {
                let list = file_list_display.lock();
                let idx = value as usize;
                if list.is_empty() {
                    return "(no models)".to_string();
                }
                let clamped = idx.min(list.len() - 1);
                std::path::Path::new(&list[clamped])
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| format!("#{}", clamped))
            }))
            .with_callback({
                // Dummy callback - actual loading is handled in process()
                Arc::new(|_| {})
            }),
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
