//! Plugin parameters for the stereo width tool (warmth-width-depth.md
//! §6.2).
//!
//! Every default is transparent: width 100 %, mono-maker off, widening
//! off, balance centre, rotation 0°, both audition toggles off. At those
//! values the plugin is a bit-exact passthrough (`tests/stereo.rs`), so
//! inserting it changes nothing until something is dialled in.
//!
//! The ids are the control API's and the skills' param keys; renaming
//! one is a state migration (`ResonancePlugin::param_renames`).

use std::sync::Arc;

use resonance_plugin::formatters::{
    s2v_f32_hz, s2v_f32_percentage, v2s_f32_hz, v2s_f32_percent,
};
use resonance_plugin::*;

use crate::dsp::{MonoSlope, WidenMode};

pub const PARAM_COUNT: usize = 11;

/// `mono_below` at or under this frequency is the Off position.
pub const MONO_BELOW_OFF_HZ: f32 = 20.0;
/// `focus_high` at or above this frequency leaves the top open (no
/// low-pass on the widened component).
pub const FOCUS_HIGH_OPEN_HZ: f32 = 20_000.0;

/// Stable indices into [`StereoParams::param_at`], in host order.
pub mod index {
    pub const WIDTH: usize = 0;
    pub const MONO_BELOW: usize = 1;
    pub const MONO_SLOPE: usize = 2;
    pub const WIDEN_MODE: usize = 3;
    pub const WIDEN_AMOUNT: usize = 4;
    pub const FOCUS_LOW: usize = 5;
    pub const FOCUS_HIGH: usize = 6;
    pub const BALANCE: usize = 7;
    pub const ROTATION: usize = 8;
    pub const SOLO_SIDE: usize = 9;
    pub const MONO_CHECK: usize = 10;
}

pub struct StereoParams {
    /// M/S side gain, 0..2 (0 % = mono, 100 % = unchanged, 200 % = side
    /// doubled, +6 dB of S/M).
    pub width: FloatParam,
    /// Mono-maker corner in Hz: side content below it is removed, so the
    /// bass folds to mono. At or below [`MONO_BELOW_OFF_HZ`] it is off.
    pub mono_below: FloatParam,
    /// Mono-maker slope, a [`MonoSlope`] index (6/12/24 dB/oct).
    pub mono_slope: IntParam,
    /// Widening algorithm, a [`WidenMode`] index.
    pub widen_mode: IntParam,
    /// How much the widening mode does, 0..1. Per mode: Decorrelate =
    /// side amount, Diffuse = all-pass spread, Micro-shift = level of the
    /// detuned voices, Haas = delay time (1–30 ms).
    pub widen_amount: FloatParam,
    /// Lower edge of the band the widening acts on (Hz); bass below it
    /// stays dry and centred. Haas uses it as its low exclude.
    pub focus_low: FloatParam,
    /// Upper edge of the widened band (Hz); at [`FOCUS_HIGH_OPEN_HZ`] the
    /// top is left open.
    pub focus_high: FloatParam,
    /// Stereo balance −1..1 (centre = unity on both sides).
    pub balance: FloatParam,
    /// Stereo rotation in degrees, −45..45 (+45° puts a centred source
    /// hard right).
    pub rotation: FloatParam,
    /// Audition: hear only the side signal (as `S, −S`).
    pub solo_side: BoolParam,
    /// Audition: hear the mono fold-down `(L+R)/2` on both sides.
    pub mono_check: BoolParam,
}

/// A parameter with its concrete type, for the editor's widget choice.
pub enum ParamRef<'a> {
    Float(&'a FloatParam),
    Int(&'a IntParam),
    Bool(&'a BoolParam),
}

impl StereoParams {
    pub fn param_ref(&self, i: usize) -> ParamRef<'_> {
        use index::*;
        match i {
            WIDTH => ParamRef::Float(&self.width),
            MONO_BELOW => ParamRef::Float(&self.mono_below),
            MONO_SLOPE => ParamRef::Int(&self.mono_slope),
            WIDEN_MODE => ParamRef::Int(&self.widen_mode),
            WIDEN_AMOUNT => ParamRef::Float(&self.widen_amount),
            FOCUS_LOW => ParamRef::Float(&self.focus_low),
            FOCUS_HIGH => ParamRef::Float(&self.focus_high),
            BALANCE => ParamRef::Float(&self.balance),
            ROTATION => ParamRef::Float(&self.rotation),
            SOLO_SIDE => ParamRef::Bool(&self.solo_side),
            MONO_CHECK => ParamRef::Bool(&self.mono_check),
            _ => ParamRef::Float(&self.width),
        }
    }

    pub fn param_at(&self, i: usize) -> &dyn Param {
        match self.param_ref(i) {
            ParamRef::Float(p) => p,
            ParamRef::Int(p) => p,
            ParamRef::Bool(p) => p,
        }
    }

    /// The current widening mode.
    pub fn widen_mode(&self) -> WidenMode {
        WidenMode::from_index(self.widen_mode.value())
    }

    /// The current mono-maker slope.
    pub fn mono_slope(&self) -> MonoSlope {
        MonoSlope::from_index(self.mono_slope.value())
    }
}

fn format_mono_below() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    let hz = v2s_f32_hz();
    Arc::new(move |v: f32| if v <= MONO_BELOW_OFF_HZ { "Off".to_string() } else { hz(v) })
}

fn format_focus_high() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    let hz = v2s_f32_hz();
    Arc::new(move |v: f32| if v >= FOCUS_HIGH_OPEN_HZ { "Open".to_string() } else { hz(v) })
}

/// Parses a frequency, plus the one word the param displays at its end
/// stop (`Off` → `off_value`).
fn parse_hz_or(word: &'static str, off_value: f32) -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    let hz = s2v_f32_hz();
    Arc::new(move |s: &str| {
        if s.trim().eq_ignore_ascii_case(word) {
            Some(off_value)
        } else {
            hz(s)
        }
    })
}

fn format_balance() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v: f32| {
        let pct = (v * 100.0).round();
        if pct == 0.0 {
            "C".to_string()
        } else if pct < 0.0 {
            format!("L {:.0}%", -pct)
        } else {
            format!("R {:.0}%", pct)
        }
    })
}

/// Reads `C`, `L 30%`, `R 30%`, or a signed percentage.
fn parse_balance() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|s: &str| {
        let t = s.trim().to_ascii_uppercase();
        if t == "C" {
            return Some(0.0);
        }
        let (sign, rest) = if let Some(r) = t.strip_prefix('L') {
            (-1.0, r)
        } else if let Some(r) = t.strip_prefix('R') {
            (1.0, r)
        } else {
            (1.0, t.as_str())
        };
        rest.trim().trim_end_matches('%').trim().parse::<f32>().ok().map(|p| sign * p / 100.0)
    })
}

fn format_degrees() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v: f32| format!("{v:+.1}°"))
}

fn parse_degrees() -> Arc<dyn Fn(&str) -> Option<f32> + Send + Sync> {
    Arc::new(|s: &str| s.trim().trim_end_matches('°').trim().parse::<f32>().ok())
}

impl Default for StereoParams {
    fn default() -> Self {
        Self {
            width: FloatParam::new("width", "Width", 1.0, FloatRange::Linear { min: 0.0, max: 2.0 })
                .with_unit("%")
                .with_value_to_string(v2s_f32_percent(0))
                .with_string_to_value(s2v_f32_percentage()),

            mono_below: FloatParam::new(
                "mono_below",
                "Mono Below",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 500.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_value_to_string(format_mono_below())
            .with_string_to_value(parse_hz_or("off", 0.0)),

            mono_slope: IntParam::new(
                "mono_slope",
                "Mono Slope",
                MonoSlope::Db12.index(),
                IntRange::Linear {
                    min: 0,
                    max: MonoSlope::LABELS.len() as i32 - 1,
                },
            )
            .with_choices(MonoSlope::LABELS),

            widen_mode: IntParam::new(
                "widen_mode",
                "Widen Mode",
                WidenMode::Off.index(),
                IntRange::Linear {
                    min: 0,
                    max: WidenMode::LABELS.len() as i32 - 1,
                },
            )
            .with_choices(WidenMode::LABELS),

            widen_amount: FloatParam::new(
                "widen_amount",
                "Widen Amount",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%")
            .with_value_to_string(v2s_f32_percent(0))
            .with_string_to_value(s2v_f32_percentage()),

            focus_low: FloatParam::new(
                "focus_low",
                "Focus Low",
                150.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 2_000.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(v2s_f32_hz())
            .with_string_to_value(s2v_f32_hz()),

            focus_high: FloatParam::new(
                "focus_high",
                "Focus High",
                FOCUS_HIGH_OPEN_HZ,
                FloatRange::Skewed {
                    min: 1_000.0,
                    max: FOCUS_HIGH_OPEN_HZ,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_value_to_string(format_focus_high())
            .with_string_to_value(parse_hz_or("open", FOCUS_HIGH_OPEN_HZ)),

            balance: FloatParam::new(
                "balance",
                "Balance",
                0.0,
                FloatRange::Linear { min: -1.0, max: 1.0 },
            )
            .with_value_to_string(format_balance())
            .with_string_to_value(parse_balance()),

            rotation: FloatParam::new(
                "rotation",
                "Rotation",
                0.0,
                FloatRange::Linear {
                    min: -45.0,
                    max: 45.0,
                },
            )
            .with_value_to_string(format_degrees())
            .with_string_to_value(parse_degrees()),

            solo_side: BoolParam::new("solo_side", "Solo Side", false),
            mono_check: BoolParam::new("mono_check", "Mono Check", false),
        }
    }
}
