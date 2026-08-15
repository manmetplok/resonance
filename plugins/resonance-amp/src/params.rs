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
    ///
    /// Visible, like resonance-ir's identical `file_select` — see the
    /// declaration below for why it stopped being `.hidden()`.
    pub file_select: IntParam,

    /// Shared file list the header browser and the loader thread index
    /// with `file_select`.
    pub file_list: Arc<Mutex<Vec<String>>>,

    pub input_gain: FloatParam,

    pub output_gain: FloatParam,
}

/// How many parameters this plugin exposes.
///
/// Declared next to the list it counts, so the two cannot drift.
pub const PARAM_COUNT: usize = 3;

impl AmpParams {
    /// The exposed parameters in host order.
    ///
    /// One ordered list, read by both `ResonancePlugin::param` and the
    /// editor's preset bar. The plugin used to spell the order out in a
    /// `match` of its own, which is the "call site restates what params.rs
    /// declares" shape this audit is removing (ba todo #1358).
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            1 => &self.input_gain,
            2 => &self.output_gain,
            // Index 0, and anything out of range, is the model selector.
            _ => &self.file_select,
        }
    }
}

impl Default for AmpParams {
    fn default() -> Self {
        Self {
            model_path: Arc::new(Mutex::new(String::new())),
            file_list: Arc::new(Mutex::new(Vec::new())),
            // Not `.hidden()` (ba todo #1283, audit finding A6). The
            // identical parameter in resonance-ir is visible, and there
            // is nothing about switching the loaded model that a host —
            // or the app's own `track.plugin_params` — should be kept
            // from: it is the single most consequential choice this
            // plugin offers.
            //
            // Hiding it never protected anything either. `hidden` is a
            // display hint, not a storage switch: the CLAP bridge always
            // wrote every param to plugin state, and since ba todo #1290
            // the engine reports hidden params to the app *flagged*
            // rather than dropping them — dropping them was what cost
            // them on save, because `ProjectPlugin.params` is derived
            // from the app mirror. So unhiding changes exactly what the
            // finding asks for (the host's param list, the generic
            // parameter panel and the automation-lane picker all stop
            // skipping it) and nothing about persistence.
            //
            // Stepped and 1000 values wide, which is deliberately past
            // the engine's `MAX_CHOICE_STEPS` (64): `ParamInfo.choices`
            // does not walk it, so a visible selector costs no per-query
            // label enumeration. The value is an index into `file_list`,
            // whose contents depend on the directory the model was
            // loaded from, so there is no fixed label set to publish.
            file_select: IntParam::new(
                "file_select",
                "Model Select",
                0,
                IntRange::Linear {
                    min: 0,
                    max: MAX_FILE_INDEX,
                },
            ),
            // Smoothers live on the plugin struct, not here, because
            // sharing `Arc<AmpParams>` with the editor thread forbids
            // `&mut` access through the Arc.
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
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}
