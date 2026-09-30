/// Plugin parameters: input/output gain, the model reference, and the
/// model selector.
use parking_lot::Mutex;
use resonance_plugin::*;
use std::sync::Arc;

use crate::library::SharedLibrary;
use crate::model_ref::{ModelRef, ModelState, ModelStatus};

/// Highest `file_select` value: the library's last slot.
pub const MAX_FILE_INDEX: i32 = resonance_common::nam_library::MAX_SLOT as i32;

pub struct AmpParams {
    /// The model reference that plays (plugin state v2): written only once
    /// a load succeeds, or verbatim when the model it names is missing.
    pub model_ref: Arc<Mutex<ModelRef>>,

    /// A reference a state load delivered that has not been resolved yet:
    /// `initialize` resolves it when inactive, the loader thread when
    /// active. `save_state` persists it until then (it is what is about
    /// to play), and it wins over the slot the same state's `file_select`
    /// asked for.
    pub pending_ref: Arc<Mutex<Option<ModelRef>>>,

    /// What the instance is playing, or why it is not.
    pub status: Arc<Mutex<ModelStatus>>,

    /// The model library shared by every amp in the process.
    pub library: Arc<SharedLibrary>,

    /// This instance's key in the library's usage registry.
    pub instance_id: u64,

    /// The model selector: a **stable slot** in the library's slot table
    /// (nam-model-library.md §5.1), not a position in a directory listing,
    /// so adding or deleting models never changes what a preset or an
    /// automation lane recalls.
    ///
    /// Visible, like resonance-ir's `file_select` — see the declaration
    /// below for why it stopped being `.hidden()`.
    pub file_select: IntParam,

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

    /// Parameters over a given shared library (tests pass their own).
    pub fn with_library(library: Arc<SharedLibrary>) -> Self {
        let status = Arc::new(Mutex::new(ModelStatus::default()));
        Self {
            model_ref: Arc::new(Mutex::new(ModelRef::default())),
            pending_ref: Arc::new(Mutex::new(None)),
            file_select: file_select_param(library.clone(), status.clone()),
            status,
            library,
            instance_id: crate::library::next_instance_id(),
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

/// The text a `file_select` value shows: the slot's model name,
/// `"(empty)"`, `"Missing: <name>"` for the value an instance's missing
/// model was saved at, or `"External: <name>"` for the value parked while
/// a model from outside the library plays. Non-blocking: a busy library
/// reads as `"slot N"`. Hosts call this on the main thread, never the
/// audio thread.
pub fn slot_text(library: &SharedLibrary, status: &Mutex<ModelStatus>, value: i32) -> String {
    if let Some(st) = status.try_lock() {
        if let ModelState::Missing { name, at_slot, .. } = &st.state {
            if *at_slot == value {
                return format!("Missing: {name}");
            }
        }
        if st.external && st.state == ModelState::Loaded && st.external_slot == Some(value) {
            return format!("External: {}", st.name);
        }
    }
    match library.try_read() {
        Some(lib) => match lib.by_slot(value.max(0) as u32) {
            Some(e) => e.name.clone(),
            None => "(empty)".to_string(),
        },
        None => format!("slot {value}"),
    }
}

/// The slot a typed or host-sent text names: a number (`"12"`, `"slot 12"`),
/// else a model name or id prefix resolved by the library
/// (`Library::find`).
pub fn slot_from_text(library: &SharedLibrary, text: &str) -> Option<i32> {
    let t = text.trim();
    let digits = t.strip_prefix("slot ").unwrap_or(t);
    if let Ok(n) = digits.parse::<i32>() {
        return (0..=MAX_FILE_INDEX).contains(&n).then_some(n);
    }
    let lib = library.try_read()?;
    lib.find(t).and_then(|e| e.slot).map(|s| s as i32)
}

fn file_select_param(library: Arc<SharedLibrary>, status: Arc<Mutex<ModelStatus>>) -> IntParam {
    let lib_for_text = library.clone();
    // Not `.hidden()` (ba todo #1283, audit finding A6). The identical
    // parameter in resonance-ir is visible, and there is nothing about
    // switching the loaded model that a host — or the app's own
    // `track.plugin_params` — should be kept from: it is the single most
    // consequential choice this plugin offers.
    //
    // Hiding it never protected anything either. `hidden` is a display
    // hint, not a storage switch: the CLAP bridge always wrote every param
    // to plugin state, and since ba todo #1290 the engine reports hidden
    // params to the app *flagged* rather than dropping them.
    //
    // Stepped and 1000 values wide, which is deliberately past the
    // engine's `MAX_CHOICE_STEPS` (64): `ParamInfo.choices` does not walk
    // it, so a visible selector costs no per-query label enumeration. The
    // value is a library slot; its text is the slot's model name, which is
    // how the control API reads which model is loaded (§9.1).
    IntParam::new(
        "file_select",
        "Model Select",
        0,
        IntRange::Linear {
            min: 0,
            max: MAX_FILE_INDEX,
        },
    )
    .with_value_to_string(Arc::new(move |v| slot_text(&lib_for_text, &status, v)))
    .with_string_to_value(Arc::new(move |text| slot_from_text(&library, text)))
}

impl Default for AmpParams {
    fn default() -> Self {
        Self::with_library(crate::library::shared())
    }
}
