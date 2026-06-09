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
use crate::smoother::{Smoother, SmoothingStyle};

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
    /// with continuous parameters must feed a [`Smoother`] from the
    /// current param value at the start of every `process()` call
    /// (`smoother.set_target(param.value())` — see
    /// `ReverbSmoothers::update_targets` in resonance-reverb for the
    /// canonical block-rate pattern), or smooth implicitly through their
    /// own envelopes/ramps (e.g. a compressor's attack/release stage).
    /// A plugin that multiplies a raw param value straight into the
    /// signal will zipper/click under host automation.
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
    /// Whether this parameter is hidden from the host.
    fn is_hidden(&self) -> bool {
        false
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
    value: AtomicU32,
    pub smoother: Smoother,
    unit: &'static str,
    value_to_string: Option<Arc<dyn Fn(f32) -> String + Send + Sync>>,
    string_to_value: Option<Arc<dyn Fn(&str) -> Option<f32> + Send + Sync>>,
    hidden: bool,
}

impl FloatParam {
    pub fn new(id: &'static str, name: &'static str, default: f32, range: FloatRange) -> Self {
        Self {
            id,
            name,
            default,
            range,
            value: AtomicU32::new(default.to_bits()),
            smoother: Smoother::new(SmoothingStyle::None),
            unit: "",
            value_to_string: None,
            string_to_value: None,
            hidden: false,
        }
    }

    pub fn with_smoother(mut self, style: SmoothingStyle) -> Self {
        self.smoother = Smoother::new(style);
        self
    }

    pub fn with_unit(mut self, unit: &'static str) -> Self {
        self.unit = unit;
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
        if !v.is_finite() {
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
            if !self.unit.is_empty() && !s.contains(self.unit) {
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
    fn is_hidden(&self) -> bool {
        self.hidden
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
    hidden: bool,
}

impl IntParam {
    pub fn new(id: &'static str, name: &'static str, default: i32, range: IntRange) -> Self {
        Self {
            id,
            name,
            default,
            range,
            value: AtomicI32::new(default),
            hidden: false,
        }
    }

    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
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
        if !v.is_finite() {
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
        format!("{}", value.round() as i32)
    }
    fn parse(&self, text: &str) -> Option<f64> {
        text.trim().parse::<i32>().ok().map(|v| v as f64)
    }
    fn is_hidden(&self) -> bool {
        self.hidden
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
}

impl BoolParam {
    pub fn new(id: &'static str, name: &'static str, default: bool) -> Self {
        Self {
            id,
            name,
            default,
            value: AtomicBool::new(default),
        }
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
    fn is_stepped(&self) -> bool {
        true
    }
}
