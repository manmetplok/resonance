use parking_lot::Mutex;
/// Plugin parameters: dry/wet mix, output gain, persisted IR path, and file selector.
///
/// The params here only store atomic current values and shared paths
/// — no smoothers. Per-parameter smoothing lives in [`IrSmoothers`]
/// below, which the plugin owns directly (not via `Arc`) so the audio
/// thread can mutate smoother state through `&mut self`.
use resonance_plugin::*;
use std::sync::Arc;

use crate::dsp::{LatencyMode, LATENCY_MODE_LABELS};

pub const MAX_FILE_INDEX: i32 = 999;

/// Linear-gain bounds of the output trim: 0.1 is -20 dB, 10.0 is +20 dB,
/// so the range is symmetric about unity *in dB* — the geometric, not the
/// arithmetic, middle of `0.1..=10.0` is 1.0.
pub const OUTPUT_GAIN_MIN: f32 = 0.1;
pub const OUTPUT_GAIN_MAX: f32 = 10.0;

/// Skew factor for a [`FloatRange::Skewed`] over `min..=max` that puts
/// `value` at `travel` of the control's arc.
///
/// # Why this exists (ba todo #1345)
///
/// This param used to declare `gain_skew_factor(-20.0, 20.0)`. That
/// helper is `-2 * |min_db| / (max_db - min_db)`, which for any range
/// symmetric in dB is simply `-1.0` — an exponent of `2^-1 = 0.5`, i.e.
/// "square-root the linear position", regardless of how wide the range
/// actually is. It is a heuristic that bunches the low end; it says
/// nothing about where unity lands. Applied to the *linear* gain range
/// `0.1..=10.0` it put unity at 30 % of the dial and +8.2 dB at half
/// travel. Nobody noticed because the editor drew its own hardcoded
/// logarithmic arc and threw the declaration away until ba todo #1284
/// made the declaration authoritative.
///
/// A gain trim wants unity at dial centre, so the factor is derived from
/// that requirement instead of guessed. [`FloatRange::normalize`] maps
/// `travel = linear^(2^factor)` where `linear = (value - min) / (max -
/// min)`, so solving for the exponent gives
/// `factor = log2( ln(travel) / ln(linear) )`.
///
/// Only the dial mapping moves: the parameter's plain domain stays
/// linear gain over the same bounds, which is what the DSP multiplies by
/// and what project/preset state stores (`params_to_json` writes
/// `get_plain`), so no saved project or preset changes gain.
///
/// Degenerate inputs (`value` on either bound, an empty range, a `travel`
/// of 0 or 1) have no finite solution; those fall back to `0.0`, the
/// no-skew factor.
pub fn skew_placing_value_at(min: f32, max: f32, value: f32, travel: f32) -> f32 {
    let span = max - min;
    if span.abs() < f32::EPSILON {
        return 0.0;
    }
    let linear = (value - min) / span;
    if linear <= 0.0 || linear >= 1.0 || travel <= 0.0 || travel >= 1.0 {
        return 0.0;
    }
    let factor = (travel.ln() / linear.ln()).log2();
    if factor.is_finite() {
        factor
    } else {
        0.0
    }
}

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

    /// Convolution latency mode — see [`crate::dsp::LatencyMode`].
    ///
    /// A *parameter*, not an editor-only switch (ba todo #1300, audit
    /// finding I1): the block size is the plugin's reported latency, and
    /// making it a parameter is what puts it in the editor, in a host
    /// automation lane and behind `track.set_plugin_param` at once.
    ///
    /// Reading it is not the same as applying it — the block size can only
    /// change while the plugin is deactivated (CLAP only allows a reported
    /// latency to change then, and the engine's delay lines are reallocated
    /// with it), so `lib.rs` pushes the new latency to the host and applies
    /// the change in `initialize()` on the reactivation that follows.
    pub latency_mode: IntParam,
}

/// How many parameters this plugin exposes.
///
/// Declared next to the list it counts, so the two cannot drift.
pub const PARAM_COUNT: usize = 4;

impl IrParams {
    /// The exposed parameters in host order.
    ///
    /// One ordered list, read by both `ResonancePlugin::param` and the
    /// editor's preset bar, rather than a `match` restated per call site
    /// (ba todo #1358).
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            1 => &self.dry_wet,
            2 => &self.output_gain,
            3 => &self.latency_mode,
            // Index 0, and anything out of range, is the file selector.
            _ => &self.file_select,
        }
    }
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
            )
            // An index into this machine's directory listing: presets carry
            // `ir_path` instead.
            .excluded_from_presets(),
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
                    min: OUTPUT_GAIN_MIN,
                    max: OUTPUT_GAIN_MAX,
                    // Unity at dial centre, the convention for a gain
                    // trim — see `skew_placing_value_at`.
                    factor: skew_placing_value_at(OUTPUT_GAIN_MIN, OUTPUT_GAIN_MAX, 1.0, 0.5),
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
            latency_mode: IntParam::new(
                "latency_mode",
                "Latency Mode",
                LatencyMode::default().index(),
                IntRange::Linear {
                    min: 0,
                    max: LATENCY_MODE_LABELS.len() as i32 - 1,
                },
            )
            // Display *and* parse come off this one table, so every
            // surface — the editor's picker, a host automation lane,
            // `track.plugin_params` — reads the mode's name.
            .with_choices(LATENCY_MODE_LABELS),
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

impl Default for IrSmoothers {
    fn default() -> Self {
        Self::new()
    }
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
