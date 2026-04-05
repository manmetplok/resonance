/// Plugin parameters: dry/wet mix, output gain, persisted IR path, and file selector.

use nih_plug::prelude::*;
use nih_plug_iced::IcedState;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::editor;

pub const MAX_FILE_INDEX: i32 = 999;

#[derive(Params)]
pub struct IrParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<IcedState>,

    #[persist = "ir-path"]
    pub ir_path: Arc<Mutex<String>>,

    /// File selector index exposed as a DAW parameter.
    /// The host can automate this to switch between .wav files
    /// found in the same directory as the loaded IR.
    #[id = "file_select"]
    pub file_select: IntParam,

    #[id = "dry_wet"]
    pub dry_wet: FloatParam,

    #[id = "output_gain"]
    pub output_gain: FloatParam,
}

impl Default for IrParams {
    fn default() -> Self {
        let file_list: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let file_list_display = file_list.clone();

        Self {
            editor_state: editor::default_state(),
            ir_path: Arc::new(Mutex::new(String::new())),
            file_select: IntParam::new(
                "IR Select",
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
                    return "(no IRs)".to_string();
                }
                let clamped = idx.min(list.len() - 1);
                std::path::Path::new(&list[clamped])
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| format!("#{}", clamped))
            }))
            .with_callback(Arc::new(|_| {})),
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
