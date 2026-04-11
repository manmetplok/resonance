/// Plugin parameters: dry/wet mix, output gain, persisted IR path, and file selector.
///
/// The params here only store atomic current values and shared paths
/// — no smoothers. Per-parameter smoothing lives in [`IrSmoothers`]
/// below, which the plugin owns directly (not via `Arc`) so the audio
/// thread can mutate smoother state through `&mut self`.

use resonance_plugin::*;
use parking_lot::Mutex;
use std::sync::Arc;

pub const MAX_FILE_INDEX: i32 = 999;

pub struct IrParams {
    /// Persisted IR file path (not a DAW parameter, saved/loaded via custom state).
    pub ir_path: Arc<Mutex<String>>,

    /// File selector index exposed as a DAW parameter.
    /// The host can automate this to switch between .wav files
    /// found in the same directory as the loaded IR.
    pub file_select: IntParam,

    /// Shared file list used by both the display closure and the plugin.
    pub file_list: Arc<Mutex<Vec<String>>>,

    pub dry_wet: FloatParam,

    pub output_gain: FloatParam,
}

impl Default for IrParams {
    fn default() -> Self {
        Self {
            ir_path: Arc::new(Mutex::new(String::new())),
            file_list: Arc::new(Mutex::new(Vec::new())),
            file_select: IntParam::new(
                "file_select",
                "IR Select",
                0,
                IntRange::Linear {
                    min: 0,
                    max: MAX_FILE_INDEX,
                },
            ),
            dry_wet: FloatParam::new(
                "dry_wet",
                "Dry/Wet",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
            output_gain: FloatParam::new(
                "output_gain",
                "Output Gain",
                1.0,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 10.0,
                    factor: FloatRange::gain_skew_factor(-20.0, 20.0),
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}

/// Audio-thread-only smoothers. Lives outside the shared `Arc<IrParams>`
/// so the audio thread can mutate smoother state through `&mut self`
/// without fighting the editor's shared reference.
pub struct IrSmoothers {
    pub dry_wet: Smoother,
    pub output_gain: Smoother,
}

impl IrSmoothers {
    pub fn new() -> Self {
        Self {
            dry_wet: Smoother::new(SmoothingStyle::Linear(50.0)),
            output_gain: Smoother::new(SmoothingStyle::Logarithmic(50.0)),
        }
    }

    /// Call once on `initialize` — updates sample rate on every
    /// smoother and seeds them with the current param values so the
    /// first block doesn't ramp from zero.
    pub fn prepare(&mut self, sample_rate: f32, params: &IrParams) {
        self.dry_wet.set_sample_rate(sample_rate);
        self.output_gain.set_sample_rate(sample_rate);
        self.dry_wet.reset(params.dry_wet.value());
        self.output_gain.reset(params.output_gain.value());
    }

    /// Push the current atomic param values as smoother targets at
    /// the start of each block.
    pub fn retarget_from(&mut self, params: &IrParams) {
        self.dry_wet.set_target(params.dry_wet.value());
        self.output_gain.set_target(params.output_gain.value());
    }
}
