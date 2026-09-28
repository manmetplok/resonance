//! Plugin parameters for the character plugin (warmth-width-depth.md
//! §6.1).
//!
//! Every parameter here is also a control in the editor (the
//! param-first, dual-surface rule); `tests/editor_param_binding.rs`
//! holds the control strip to that.
//!
//! # `tape_quality` is deliberately absent
//!
//! §6.1 / decision D2 add a `tape_quality` switch (Standard / HQ), with HQ
//! (a Jiles-Atherton hysteresis stage) landing in slice W6b. This slice
//! ships only Standard, so the switch is **not declared yet**: a one-value
//! parameter would be a control that does nothing, and a two-value one
//! would offer an HQ that does not exist.
//!
//! Leaving it out is also what keeps saved state forward-compatible.
//! State is matched by string id, so a project saved by this build simply
//! has no `tape_quality` key; when W6b declares the parameter with
//! Standard as its default (index 0), such a project loads as Standard and
//! renders exactly as it did here. W6b must keep Standard at index 0 and
//! as the default for that to hold.

use std::sync::Arc;

use resonance_dsp::OversampleFactor;
use resonance_plugin::formatters::{s2v_f32_percentage, v2s_f32_db, v2s_f32_percent};
use resonance_plugin::*;

pub const PARAM_COUNT: usize = 11;

/// The five voicings, in the order §6.1 lists them. The discriminant is
/// the `mode` parameter's plain value, so the order is part of the saved
/// state and must never be reshuffled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Mode {
    Tube = 0,
    Tape = 1,
    Transformer = 2,
    Console = 3,
    Warm = 4,
}

impl Mode {
    /// Display labels, indexed by discriminant.
    pub const LABELS: [&'static str; 5] = ["Tube", "Tape", "Transformer", "Console", "Warm"];

    pub const ALL: [Mode; 5] = [
        Mode::Tube,
        Mode::Tape,
        Mode::Transformer,
        Mode::Console,
        Mode::Warm,
    ];

    pub fn from_int(v: i32) -> Self {
        match v {
            1 => Mode::Tape,
            2 => Mode::Transformer,
            3 => Mode::Console,
            4 => Mode::Warm,
            _ => Mode::Tube,
        }
    }

    pub fn label(self) -> &'static str {
        Self::LABELS[self as usize]
    }

    /// Whether `bias` shapes this mode. Console is the one odd-only,
    /// clean voicing and ignores it (the editor greys the knob).
    pub fn uses_bias(self) -> bool {
        !matches!(self, Mode::Console)
    }
}

/// Tape speed choices, in inches per second, indexed by the `speed`
/// parameter's plain value.
pub const SPEED_IPS: [f32; 3] = [7.5, 15.0, 30.0];
pub const SPEED_LABELS: [&str; 3] = ["7.5 ips", "15 ips", "30 ips"];

pub fn speed_ips(index: i32) -> f32 {
    SPEED_IPS[index.clamp(0, SPEED_IPS.len() as i32 - 1) as usize]
}

pub struct ColorParams {
    /// Voicing ([`Mode`]).
    pub mode: IntParam,
    /// How hard the signal hits the curve, 0..1. The mapping to an
    /// internal gain is per mode (see `dsp::voicing`); 0 in Console is
    /// the identity.
    pub drive: FloatParam,
    /// Asymmetry, 0..1: the curve's input offset in Tube / Tape /
    /// Transformer, the even-curve amount in Warm, ignored in Console.
    pub bias: FloatParam,
    /// Tilt of the *drive*, in dB: positive drives the lows harder,
    /// negative the highs (a pre-/de-emphasis pair around the curve, so
    /// at small signal it is flat).
    pub response: FloatParam,
    /// Output tilt of the wet signal, in dB: positive is brighter.
    pub tone: FloatParam,
    /// Dry/wet blend, 0..1.
    pub mix: FloatParam,
    /// Match the wet signal's K-weighted RMS to the input's. On by
    /// default: louder reads as warmer, so an unmatched saturator cannot
    /// be judged (warmth-width-depth.md §1).
    pub auto_gain: BoolParam,
    /// Output trim in dB, after the mix.
    pub output: FloatParam,
    /// IIR oversampling of the curve, Off / 2x / 4x. Latency-free
    /// (decision D3), so changing it never changes the reported latency.
    pub oversample: IntParam,
    /// Tape speed ([`SPEED_IPS`]); Tape mode only.
    pub speed: IntParam,
    /// Wow and flutter amount, 0..1; Tape mode only. 0 bypasses it
    /// entirely (no delay, no interpolation).
    pub flutter: FloatParam,
}

impl ColorParams {
    pub fn param_at(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.mode,
            1 => &self.drive,
            2 => &self.bias,
            3 => &self.response,
            4 => &self.tone,
            5 => &self.mix,
            6 => &self.auto_gain,
            7 => &self.output,
            8 => &self.oversample,
            9 => &self.speed,
            10 => &self.flutter,
            _ => &self.mode,
        }
    }

    pub fn all(&self) -> Vec<&dyn Param> {
        (0..PARAM_COUNT).map(|i| self.param_at(i)).collect()
    }

    pub fn mode(&self) -> Mode {
        Mode::from_int(self.mode.value())
    }

    pub fn oversample_factor(&self) -> OversampleFactor {
        OversampleFactor::from_int(self.oversample.value())
    }
}

fn percent() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    v2s_f32_percent(0)
}

/// `+3.0 dB` / `-3.0 dB` / `0.0 dB`: a signed readout for the tilts.
fn signed_db() -> Arc<dyn Fn(f32) -> String + Send + Sync> {
    Arc::new(|v: f32| {
        if v.abs() < 0.05 {
            "0.0 dB".to_string()
        } else {
            format!("{:+.1} dB", v)
        }
    })
}

fn unit_percent(id: &'static str, name: &'static str, default: f32) -> FloatParam {
    FloatParam::new(id, name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_unit("%")
        .with_value_to_string(percent())
        .with_string_to_value(s2v_f32_percentage())
}

impl Default for ColorParams {
    fn default() -> Self {
        Self {
            mode: IntParam::new(
                "mode",
                "Mode",
                Mode::Tube as i32,
                IntRange::Linear {
                    min: 0,
                    max: Mode::LABELS.len() as i32 - 1,
                },
            )
            .with_choices(&Mode::LABELS),

            drive: unit_percent("drive", "Drive", 0.35),
            bias: unit_percent("bias", "Bias", 0.5),

            response: FloatParam::new(
                "response",
                "Response",
                0.0,
                FloatRange::Linear {
                    min: -12.0,
                    max: 12.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(signed_db()),

            tone: FloatParam::new(
                "tone",
                "Tone",
                0.0,
                FloatRange::Linear { min: -6.0, max: 6.0 },
            )
            .with_unit(" dB")
            .with_value_to_string(signed_db()),

            mix: unit_percent("mix", "Mix", 1.0),

            auto_gain: BoolParam::new("auto_gain", "Auto Gain", true),

            output: FloatParam::new(
                "output",
                "Output",
                0.0,
                FloatRange::Linear {
                    min: -24.0,
                    max: 12.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(v2s_f32_db(1)),

            oversample: IntParam::new(
                "oversample",
                "Oversample",
                OversampleFactor::X2 as i32,
                IntRange::Linear {
                    min: 0,
                    max: OversampleFactor::LABELS.len() as i32 - 1,
                },
            )
            .with_choices(&OversampleFactor::LABELS),

            speed: IntParam::new(
                "speed",
                "Speed",
                1,
                IntRange::Linear {
                    min: 0,
                    max: SPEED_LABELS.len() as i32 - 1,
                },
            )
            .with_choices(&SPEED_LABELS)
            .with_module("Tape"),

            flutter: unit_percent("flutter", "Flutter", 0.0).with_module("Tape"),
        }
    }
}
