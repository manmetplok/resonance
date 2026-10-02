//! Parameter types: FloatParam, IntParam, BoolParam and the Param trait.
//!
//! The Param structs hold `Arc<dyn Fn(...) -> ... + Send + Sync>`
//! formatter closures whose type signature clippy considers complex.
//! These are part of the public param API and naturally express
//! optional host-display hooks; aliasing them away wouldn't aid
//! readability, so we allow the lint module-wide.
#![allow(clippy::type_complexity)]

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::Arc;

use crate::range::{FloatRange, IntRange};

// ---------------------------------------------------------------------------
// Param trait -- common interface for enumeration by the CLAP bridge
// ---------------------------------------------------------------------------

/// Common parameter interface used by the CLAP bridge to enumerate and
/// interact with plugin parameters.
pub trait Param: Send + Sync {
    /// Stable string identifier (used to generate CLAP param ID).
    fn id(&self) -> &str;
    /// Human-readable name.
    fn name(&self) -> &str;
    /// Get the current value as a plain (non-normalized) f64.
    fn get_plain(&self) -> f64;
    /// Set the value from a plain (non-normalized) f64.
    ///
    /// # Smoothing contract
    ///
    /// The CLAP bridge applies host automation (`CLAP_EVENT_PARAM_VALUE`)
    /// by calling this method directly at the top of each process block —
    /// the new value lands instantly and the bridge performs **no
    /// smoothing of its own**. De-zippering is the plugin's job: plugins
    /// with continuous parameters must feed a
    /// [`crate::smoother::Smoother`] **of their own** from the current
    /// param value at the start of every `process()` call
    /// (`smoother.set_target(param.value())` — see
    /// `ReverbSmoothers::update_targets` in resonance-reverb for the
    /// canonical block-rate pattern), or smooth implicitly through their
    /// own envelopes/ramps (e.g. a compressor's attack/release stage).
    /// A plugin that multiplies a raw param value straight into the
    /// signal will zipper/click under host automation.
    ///
    /// "Of their own" is the whole design: a smoother has to live where
    /// something holds it `&mut`, which a param — shared behind an `Arc`
    /// and read through `&self` — never can.
    fn set_plain(&self, v: f64);
    /// Default value as plain f64.
    fn default_plain(&self) -> f64;
    /// Minimum value as plain f64.
    fn min_plain(&self) -> f64;
    /// Maximum value as plain f64.
    fn max_plain(&self) -> f64;
    /// Format a value for display.
    fn display(&self, value: f64) -> String;
    /// Parse a display string back to a value.
    fn parse(&self, text: &str) -> Option<f64>;
    /// Apply a value the user typed in, parsed through this parameter's
    /// own [`Param::parse`] (ba todo #1287, finding F5).
    ///
    /// Returns `false` — leaving the parameter untouched — when the text
    /// is not something this parameter understands, so an editor can
    /// reject an entry rather than resolve it to zero. Range clamping is
    /// [`Param::set_plain`]'s, which is what makes an out-of-range entry
    /// land on the nearest declared bound instead of being refused.
    ///
    /// `parse` and its `text_to_value` bridge path were implemented and
    /// unit-tested long before anything called them: there was no way to
    /// type a value into any control in any editor, so a compressor could
    /// not be set to exactly -18.0 dB.
    fn apply_typed_entry(&self, text: &str) -> bool {
        match self.parse(text) {
            Some(v) if v.is_finite() => {
                self.set_plain(v);
                true
            }
            _ => false,
        }
    }
    /// The parameter's group, as a `/`-separated path (ba todo #1289,
    /// finding X7).
    ///
    /// This is CLAP's `clap_param_info.module`: hosts use it to build the
    /// tree in their automation-lane picker, so `"Multiband/Low"` shows
    /// up nested where `""` lands in one flat list. Mastering declares
    /// ~60 params across 8 stages and wavetable 87; without a module they
    /// arrive in a host as a single undifferentiated column, even though
    /// the plugins' own editors tab them.
    ///
    /// Empty means "no group", which is what CLAP expects for a
    /// top-level parameter — that stays the default.
    fn module(&self) -> &str {
        ""
    }
    /// Whether this parameter is hidden from the host.
    fn is_hidden(&self) -> bool {
        false
    }
    /// Whether presets leave this parameter out (plugin-preset-library.md
    /// §9.2): a preset neither writes nor recalls it, so loading one keeps
    /// the current value. For controls that are session/instance state
    /// rather than sound (the amp's `file_select`, a slot into this
    /// machine's library — the model travels by content id instead).
    ///
    /// Defaults to [`Param::state_excluded`]: a parameter the state leaves
    /// out is left out of every preset too, whoever implements the trait
    /// (a preset is a form of the state). An override must keep that.
    fn preset_excluded(&self) -> bool {
        self.state_excluded()
    }
    /// Whether a host may automate this parameter — CLAP's
    /// `IS_AUTOMATABLE`, which the bridge sets for every parameter that
    /// does not opt out. A control whose every change is heavy work (the
    /// drums' `kit_select` swaps a multi-gigabyte kit) opts out: it can
    /// still be set, recalled and undone, but a host offers no lane for
    /// it.
    fn is_automatable(&self) -> bool {
        true
    }
    /// Whether this parameter is an output: the plugin writes it and
    /// nothing else may (CLAP `IS_READONLY`) — a load progress, a meter.
    /// A host or a state load writing one is ignored, it is never
    /// automatable, and it is never saved ([`Param::state_excluded`]).
    fn is_read_only(&self) -> bool {
        false
    }
    /// Whether plugin state leaves this parameter out: it is neither
    /// written nor recalled by a state or a preset (so it implies
    /// [`Param::preset_excluded`]'s effect). For a value derived from
    /// something the state carries in its own form — the drums'
    /// `kit_select` slot is this machine's library layout, and the kit
    /// travels as a content reference instead — and for every
    /// [`read-only`](Param::is_read_only) output.
    fn state_excluded(&self) -> bool {
        self.is_read_only()
    }
    /// Whether this parameter is stepped (integer/bool).
    fn is_stepped(&self) -> bool {
        false
    }
    /// Compute a stable u32 CLAP param ID from the string ID.
    fn clap_id(&self) -> u32 {
        crate::stable_hash(self.id())
    }
}

// ---------------------------------------------------------------------------
// FloatParam
// ---------------------------------------------------------------------------

pub struct FloatParam {
    id: &'static str,
    name: &'static str,
    default: f32,
    range: FloatRange,
    /// Atomic storage for thread-safe value access (bit-punned f32).
    ///
    /// Note there is deliberately no smoother here — see the smoothing
    /// contract on [`Param::set_plain`]. A param is shared behind an
    /// `Arc` and handed out as `&self`, while `Smoother::next` needs
    /// `&mut self`, so a smoother stored on a param could never advance.
    /// Every plugin owns its smoothers in its own DSP struct instead
    /// (ba todo #1288).
    value: AtomicU32,
    unit: &'static str,
    module: &'static str,
    value_to_string: Option<Arc<dyn Fn(f32) -> String + Send + Sync>>,
    string_to_value: Option<Arc<dyn Fn(&str) -> Option<f32> + Send + Sync>>,
    hidden: bool,
    preset_excluded: bool,
    /// See [`Param::is_automatable`].
    automatable: bool,
    /// See [`Param::is_read_only`].
    read_only: bool,
    /// See [`Param::state_excluded`].
    state_excluded: bool,
}

impl FloatParam {
    pub fn new(id: &'static str, name: &'static str, default: f32, range: FloatRange) -> Self {
        Self {
            id,
            name,
            default,
            range,
            value: AtomicU32::new(default.to_bits()),
            unit: "",
            module: "",
            value_to_string: None,
            string_to_value: None,
            hidden: false,
            preset_excluded: false,
            automatable: true,
            read_only: false,
            state_excluded: false,
        }
    }

    pub fn with_unit(mut self, unit: &'static str) -> Self {
        self.unit = unit;
        self
    }

    /// Put this parameter in a host-visible group — see [`Param::module`].
    /// `/`-separated for nesting: `"Multiband/Low"`.
    pub fn with_module(mut self, module: &'static str) -> Self {
        self.module = module;
        self
    }

    pub fn with_value_to_string(mut self, f: Arc<dyn Fn(f32) -> String + Send + Sync>) -> Self {
        self.value_to_string = Some(f);
        self
    }

    pub fn with_string_to_value(
        mut self,
        f: Arc<dyn Fn(&str) -> Option<f32> + Send + Sync>,
    ) -> Self {
        self.string_to_value = Some(f);
        self
    }

    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Leave this parameter out of presets — see [`Param::preset_excluded`].
    pub fn excluded_from_presets(mut self) -> Self {
        self.preset_excluded = true;
        self
    }

    /// Offer no automation lane for this parameter — see
    /// [`Param::is_automatable`].
    pub fn not_automatable(mut self) -> Self {
        self.automatable = false;
        self
    }

    /// Leave this parameter out of plugin state and presets — see
    /// [`Param::state_excluded`].
    pub fn excluded_from_state(mut self) -> Self {
        self.state_excluded = true;
        self.preset_excluded = true;
        self
    }

    /// Make this parameter an output only the plugin writes — see
    /// [`Param::is_read_only`]. Also not automatable and not saved.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self.automatable = false;
        self.state_excluded = true;
        self.preset_excluded = true;
        self
    }

    /// Get the current value (thread-safe, relaxed ordering).
    pub fn value(&self) -> f32 {
        f32::from_bits(self.value.load(Ordering::Relaxed))
    }

    /// Set the value (thread-safe).
    pub fn set_value(&self, v: f32) {
        if !v.is_finite() {
            return;
        }
        self.value.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn range(&self) -> &FloatRange {
        &self.range
    }

    /// The declared default, as a plain value.
    pub fn default_value(&self) -> f32 {
        self.default
    }

    /// The declared unit suffix (`""` when the param declares none).
    pub fn unit(&self) -> &'static str {
        self.unit
    }

    // -- normalized (0..1) view -------------------------------------------
    //
    // Every editor control moves in 0..1 travel — a knob arc, a slider
    // groove — while the parameter itself is a plain value on a possibly
    // skewed range. These three map between the two through the param's
    // *own* `FloatRange`, so a control can never follow a curve or reach
    // an endpoint the parameter does not declare. They are the contract
    // `editor_widgets::float_knob` / `float_slider` are built on, and are
    // testable without a GUI.

    /// Where the current value sits on the control's 0..1 travel.
    pub fn normalized_value(&self) -> f32 {
        self.range.normalize(self.value())
    }

    /// Where the default sits on the control's 0..1 travel (the position
    /// a double-click-to-reset returns to).
    pub fn default_normalized(&self) -> f32 {
        self.range.normalize(self.default)
    }

    /// The plain value a 0..1 control position maps to.
    pub fn plain_at_normalized(&self, normalized: f32) -> f32 {
        self.range.denormalize(normalized)
    }

    /// Move the parameter to a 0..1 control position.
    ///
    /// Landing exactly on [`FloatParam::default_normalized`] writes the
    /// declared default verbatim: a reset gesture has to produce `2.0 s`,
    /// not the `1.9999998` the curve's round trip would otherwise leave.
    pub fn set_normalized(&self, normalized: f32) {
        if normalized == self.default_normalized() {
            self.set_value(self.default);
        } else {
            self.set_value(self.plain_at_normalized(normalized));
        }
    }
}

impl Param for FloatParam {
    fn id(&self) -> &str {
        self.id
    }
    fn name(&self) -> &str {
        self.name
    }
    fn get_plain(&self) -> f64 {
        self.value() as f64
    }
    fn set_plain(&self, v: f64) {
        // An output: only the plugin writes it (through `set_value`).
        if !v.is_finite() || self.read_only {
            return;
        }
        // Clamp to the declared range so a misbehaving host or a
        // corrupt preset can't push the value beyond what the DSP code
        // is built to handle (e.g. a filter cutoff outside Nyquist
        // turning every block into NaN that propagates downstream).
        let clamped = v.clamp(self.min_plain(), self.max_plain());
        self.set_value(clamped as f32);
    }
    fn default_plain(&self) -> f64 {
        self.default as f64
    }
    fn min_plain(&self) -> f64 {
        self.range.min() as f64
    }
    fn max_plain(&self) -> f64 {
        self.range.max() as f64
    }
    fn display(&self, value: f64) -> String {
        if let Some(f) = &self.value_to_string {
            let s = f(value as f32);
            // Compare on the unit's *word*, not the string with its
            // leading space: a param declaring `" Hz"` whose formatter
            // switches to `"1.00 kHz"` above a kilohertz was getting the
            // unit appended anyway, so every host read `"1.00 kHz Hz"` —
            // and nothing could parse that back (ba todo #1287).
            let word = self.unit.trim();
            if !word.is_empty() && !s.contains(word) {
                format!("{}{}", s, self.unit)
            } else {
                s
            }
        } else if !self.unit.is_empty() {
            format!("{:.2}{}", value, self.unit)
        } else {
            format!("{:.2}", value)
        }
    }
    fn parse(&self, text: &str) -> Option<f64> {
        if let Some(f) = &self.string_to_value {
            f(text).map(|v| v as f64)
        } else {
            let text = text.trim().trim_end_matches(self.unit).trim();
            text.parse::<f64>().ok()
        }
    }
    fn module(&self) -> &str {
        self.module
    }
    fn is_hidden(&self) -> bool {
        self.hidden
    }
    fn preset_excluded(&self) -> bool {
        // A state-excluded param is never in a preset either (see the
        // trait doc), whichever builder set the flags.
        self.preset_excluded || self.state_excluded
    }
    fn is_automatable(&self) -> bool {
        self.automatable
    }
    fn is_read_only(&self) -> bool {
        self.read_only
    }
    fn state_excluded(&self) -> bool {
        self.state_excluded
    }
}

// ---------------------------------------------------------------------------
// IntParam
// ---------------------------------------------------------------------------

pub struct IntParam {
    id: &'static str,
    name: &'static str,
    default: i32,
    range: IntRange,
    value: AtomicI32,
    module: &'static str,
    /// Choice labels, when the param is an enumeration — see
    /// [`IntParam::with_choices`]. Kept alongside the formatter closures
    /// so a caller that needs the *table* (a host's enum list, the
    /// control API's `choices[]`, an editor's combo box) can read it
    /// instead of probing `display` value by value.
    choices: Option<&'static [&'static str]>,
    value_to_string: Option<Arc<dyn Fn(i32) -> String + Send + Sync>>,
    string_to_value: Option<Arc<dyn Fn(&str) -> Option<i32> + Send + Sync>>,
    hidden: bool,
    preset_excluded: bool,
    /// See [`Param::is_automatable`].
    automatable: bool,
    /// See [`Param::is_read_only`].
    read_only: bool,
    /// See [`Param::state_excluded`].
    state_excluded: bool,
}

impl IntParam {
    pub fn new(id: &'static str, name: &'static str, default: i32, range: IntRange) -> Self {
        Self {
            id,
            name,
            default,
            range,
            value: AtomicI32::new(default),
            module: "",
            choices: None,
            value_to_string: None,
            string_to_value: None,
            hidden: false,
            preset_excluded: false,
            automatable: true,
            read_only: false,
            state_excluded: false,
        }
    }

    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Leave this parameter out of presets — see [`Param::preset_excluded`].
    pub fn excluded_from_presets(mut self) -> Self {
        self.preset_excluded = true;
        self
    }

    /// Offer no automation lane for this parameter — see
    /// [`Param::is_automatable`].
    pub fn not_automatable(mut self) -> Self {
        self.automatable = false;
        self
    }

    /// Leave this parameter out of plugin state and presets — see
    /// [`Param::state_excluded`].
    pub fn excluded_from_state(mut self) -> Self {
        self.state_excluded = true;
        self.preset_excluded = true;
        self
    }

    /// Make this parameter an output only the plugin writes — see
    /// [`Param::is_read_only`]. Also not automatable and not saved.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self.automatable = false;
        self.state_excluded = true;
        self.preset_excluded = true;
        self
    }

    /// Put this parameter in a host-visible group — see [`Param::module`].
    pub fn with_module(mut self, module: &'static str) -> Self {
        self.module = module;
        self
    }

    /// Format this parameter's value for display (ba todo #1289,
    /// finding X9).
    ///
    /// Without one, an int param renders as a bare integer everywhere
    /// outside its own editor: a host automation lane for the IR plugin's
    /// cab index reads `37` instead of a cabinet name. The label tables
    /// existed already — they just lived inside the editors, where the
    /// host and the control API cannot see them.
    pub fn with_value_to_string(mut self, f: Arc<dyn Fn(i32) -> String + Send + Sync>) -> Self {
        self.value_to_string = Some(f);
        self
    }

    /// Parse a display string back to a value (the inverse of
    /// [`IntParam::with_value_to_string`]), so a host's or a user's typed
    /// entry reaches the parameter.
    pub fn with_string_to_value(
        mut self,
        f: Arc<dyn Fn(&str) -> Option<i32> + Send + Sync>,
    ) -> Self {
        self.string_to_value = Some(f);
        self
    }

    /// Declare this parameter as an enumeration over `labels`, indexed
    /// from the range's minimum.
    ///
    /// One call gives the whole fleet what it needs from a choice param:
    /// the editor's combo box, the host's display, and a parse that
    /// accepts either the label or the raw index. `labels` must be
    /// `'static` (the editors need a stable slice to render per frame
    /// without rebuilding it) and cover the declared range.
    pub fn with_choices(mut self, labels: &'static [&'static str]) -> Self {
        self.choices = Some(labels);
        let min = self.range.min();
        self.value_to_string = Some(Arc::new(move |v: i32| {
            match usize::try_from(v - min).ok().and_then(|i| labels.get(i)) {
                Some(label) => (*label).to_string(),
                // Out of table: show the number rather than a wrong
                // label, so a range/table mismatch is visible instead of
                // silently reading as the last choice.
                None => v.to_string(),
            }
        }));
        self.string_to_value = Some(Arc::new(move |text: &str| {
            let trimmed = text.trim();
            labels
                .iter()
                .position(|label| label.eq_ignore_ascii_case(trimmed))
                .and_then(|i| i32::try_from(i).ok())
                .map(|i| i + min)
                .or_else(|| trimmed.parse::<i32>().ok())
        }));
        self
    }

    /// The declared choice labels, if this parameter is an enumeration.
    pub fn choices(&self) -> Option<&'static [&'static str]> {
        self.choices
    }

    pub fn range(&self) -> &IntRange {
        &self.range
    }

    /// The declared default.
    pub fn default_value(&self) -> i32 {
        self.default
    }

    pub fn value(&self) -> i32 {
        self.value.load(Ordering::Relaxed)
    }

    pub fn set_value(&self, v: i32) {
        self.value.store(v, Ordering::Relaxed);
    }
}

impl Param for IntParam {
    fn id(&self) -> &str {
        self.id
    }
    fn name(&self) -> &str {
        self.name
    }
    fn get_plain(&self) -> f64 {
        self.value() as f64
    }
    fn set_plain(&self, v: f64) {
        // Non-finite inputs are silently ignored; finite values get
        // clamped to the declared range before truncation. Mirrors the
        // FloatParam clamp so a buggy host can't shove an int param
        // far outside its bounds either.
        // An output: only the plugin writes it (through `set_value`).
        if !v.is_finite() || self.read_only {
            return;
        }
        let clamped = v.clamp(self.min_plain(), self.max_plain());
        self.set_value(clamped.round() as i32);
    }
    fn default_plain(&self) -> f64 {
        self.default as f64
    }
    fn min_plain(&self) -> f64 {
        self.range.min() as f64
    }
    fn max_plain(&self) -> f64 {
        self.range.max() as f64
    }
    fn display(&self, value: f64) -> String {
        let v = value.round() as i32;
        match &self.value_to_string {
            Some(f) => f(v),
            None => format!("{}", v),
        }
    }
    fn parse(&self, text: &str) -> Option<f64> {
        match &self.string_to_value {
            Some(f) => f(text).map(|v| v as f64),
            None => text.trim().parse::<i32>().ok().map(|v| v as f64),
        }
    }
    fn module(&self) -> &str {
        self.module
    }
    fn is_hidden(&self) -> bool {
        self.hidden
    }
    fn preset_excluded(&self) -> bool {
        // A state-excluded param is never in a preset either (see the
        // trait doc), whichever builder set the flags.
        self.preset_excluded || self.state_excluded
    }
    fn is_automatable(&self) -> bool {
        self.automatable
    }
    fn is_read_only(&self) -> bool {
        self.read_only
    }
    fn state_excluded(&self) -> bool {
        self.state_excluded
    }
    fn is_stepped(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// BoolParam
// ---------------------------------------------------------------------------

pub struct BoolParam {
    id: &'static str,
    name: &'static str,
    default: bool,
    value: AtomicBool,
    module: &'static str,
}

impl BoolParam {
    pub fn new(id: &'static str, name: &'static str, default: bool) -> Self {
        Self {
            id,
            name,
            default,
            value: AtomicBool::new(default),
            module: "",
        }
    }

    /// Put this parameter in a host-visible group — see [`Param::module`].
    pub fn with_module(mut self, module: &'static str) -> Self {
        self.module = module;
        self
    }

    /// The declared default.
    pub fn default_value(&self) -> bool {
        self.default
    }

    pub fn value(&self) -> bool {
        self.value.load(Ordering::Relaxed)
    }

    pub fn set_value(&self, v: bool) {
        self.value.store(v, Ordering::Relaxed);
    }
}

impl Param for BoolParam {
    fn id(&self) -> &str {
        self.id
    }
    fn name(&self) -> &str {
        self.name
    }
    fn get_plain(&self) -> f64 {
        if self.value() {
            1.0
        } else {
            0.0
        }
    }
    fn set_plain(&self, v: f64) {
        self.set_value(v >= 0.5);
    }
    fn default_plain(&self) -> f64 {
        if self.default {
            1.0
        } else {
            0.0
        }
    }
    fn min_plain(&self) -> f64 {
        0.0
    }
    fn max_plain(&self) -> f64 {
        1.0
    }
    fn display(&self, value: f64) -> String {
        if value >= 0.5 {
            "On".to_string()
        } else {
            "Off".to_string()
        }
    }
    fn parse(&self, text: &str) -> Option<f64> {
        match text.trim().to_lowercase().as_str() {
            "on" | "true" | "1" | "yes" => Some(1.0),
            "off" | "false" | "0" | "no" => Some(0.0),
            _ => None,
        }
    }
    fn module(&self) -> &str {
        self.module
    }
    fn is_stepped(&self) -> bool {
        true
    }
}
