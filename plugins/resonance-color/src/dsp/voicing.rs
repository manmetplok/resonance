//! Per-mode voicing: how the `drive` and `bias` knobs map onto a
//! [`Curve`] and its input gain, and the fixed filter constants each mode
//! wraps around the curve.
//!
//! Everything here is a voicing choice, not a measurement of a machine
//! (§6.1 non-goal: no circuit emulation). The numbers the rest of the
//! crate depends on are pinned by `tests/harmonics.rs`, which measures
//! them the way §2.1 defines the targets: a 1 kHz sine at −18 dBFS.
//!
//! # The drive law
//!
//! For every mode but Console the curve runs on `u = G·x` and its output
//! is divided by `G·f'(0)`, so the small-signal gain is 1 at any drive:
//! turning `drive` up adds distortion (and compression of loud peaks),
//! not level. `G` sweeps [`DRIVE_MIN_DB`]..[`DRIVE_MAX_DB`] in dB, linear
//! in the knob. `auto_gain` then matches what level change is left.
//!
//! Console is the exception the W5 primitives already make: its `sin`
//! curve carries its own drive `k` (`sin(k·u)/k`, unity slope at any `k`),
//! and `k = 0` is the identity. The knob maps linearly to
//! `0..`[`CONSOLE_MAX_DRIVE`], so drive 0 in Console is bit-transparent.

use resonance_dsp::Curve;

use crate::params::Mode;

/// Curve input gain at `drive` = 0, for the gain-driven modes.
pub const DRIVE_MIN_DB: f32 = -12.0;
/// Curve input gain at `drive` = 1.
pub const DRIVE_MAX_DB: f32 = 24.0;

/// Console's `k` at `drive` = 1 (the curve clamps it to 4).
pub const CONSOLE_MAX_DRIVE: f32 = 4.0;

/// Tube: the bias knob is the `Curve::Tube` bias directly (0..1; the
/// wavetable's fixed Tube shaper sits at 0.5).
pub const TUBE_BIAS_SCALE: f32 = 1.0;
/// Tape: a mostly symmetric soft curve with a little asymmetry.
pub const TAPE_BIAS_SCALE: f32 = 0.35;
/// Transformer: likewise mostly odd; the character is in the LF weighting.
pub const TRANSFORMER_BIAS_SCALE: f32 = 0.35;

/// Corner of the `response` pre-/de-emphasis shelf pair.
pub const RESPONSE_CORNER_HZ: f32 = 650.0;

/// Tape head bump (at [`resonance_dsp::tape::head_bump_hz`] of the speed)
/// and the dip an octave above it.
pub const TAPE_BUMP_DB: f32 = 2.0;
pub const TAPE_DIP_DB: f32 = -1.0;
/// Tape HF loss: a fixed part and a part that deepens with HF level
/// (self-erasure), full at an HF envelope of [`TAPE_HF_REFERENCE`].
pub const TAPE_HF_STATIC_DB: f32 = -1.0;
pub const TAPE_HF_DYNAMIC_DB: f32 = -4.0;
pub const TAPE_HF_REFERENCE: f32 = 0.1;
pub const TAPE_HF_ATTACK_MS: f32 = 1.0;
pub const TAPE_HF_RELEASE_MS: f32 = 60.0;

/// Transformer: bass reaches the curve this much hotter below the corner
/// (flux ∝ V/f), and is de-emphasised by the same amount afterwards.
pub const TRANSFORMER_LF_CORNER_HZ: f32 = 180.0;
pub const TRANSFORMER_LF_BOOST_DB: f32 = 9.0;
/// The coupling's sub-sonic high-pass (it also removes the curve's DC).
pub const TRANSFORMER_SUBSONIC_HZ: f32 = 16.0;
/// The small HF resonance near the top of the band.
pub const TRANSFORMER_HF_RES_HZ: f32 = 15_000.0;
pub const TRANSFORMER_HF_RES_DB: f32 = 0.8;

/// Pivot of the `tone` tilt, and the slope of its two shelves.
pub const TONE_PIVOT_HZ: f32 = 900.0;
pub const TONE_Q: f32 = 0.5;

/// Curve input gain for a gain-driven mode, or Console's `k`.
pub fn drive_amount(mode: Mode, drive: f32) -> f32 {
    let d = if drive.is_nan() { 0.0 } else { drive.clamp(0.0, 1.0) };
    match mode {
        Mode::Console => CONSOLE_MAX_DRIVE * d,
        _ => resonance_dsp::db_to_linear(DRIVE_MIN_DB + (DRIVE_MAX_DB - DRIVE_MIN_DB) * d),
    }
}

/// The curve a mode runs at a given drive amount (see [`drive_amount`])
/// and bias knob.
pub fn curve(mode: Mode, amount: f32, bias: f32) -> Curve {
    let b = if bias.is_nan() { 0.0 } else { bias.clamp(0.0, 1.0) };
    match mode {
        Mode::Tube => Curve::Tube {
            bias: TUBE_BIAS_SCALE * b,
        },
        Mode::Tape => Curve::Tube {
            bias: TAPE_BIAS_SCALE * b,
        },
        Mode::Transformer => Curve::Tube {
            bias: TRANSFORMER_BIAS_SCALE * b,
        },
        Mode::Console => Curve::Console { drive: amount },
        Mode::Warm => Curve::Warm { amount: b },
    }
}

/// Whether the mode's curve family can be asymmetric and so needs a DC
/// blocker after it. Decided by the mode alone, never by the bias value,
/// so turning `bias` through 0 never switches a filter in or out.
/// Transformer is excluded because its sub-sonic high-pass already
/// removes the DC.
pub fn needs_dc_block(mode: Mode) -> bool {
    matches!(mode, Mode::Tube | Mode::Tape | Mode::Warm)
}

/// The static transfer function the editor draws: the mode's curve with
/// its drive law and normalisation, blended with the dry signal by `mix`.
/// It leaves out every filter (emphasis, bump, HF loss, tone), which have
/// no place on an input→output amplitude plot.
pub fn transfer(mode: Mode, drive: f32, bias: f32, mix: f32, x: f32) -> f32 {
    let amount = drive_amount(mode, drive);
    let c = curve(mode, amount, bias);
    let x64 = x as f64;
    let wet = match mode {
        Mode::Console => c.eval(x64),
        _ => {
            let g = amount as f64;
            let shaped = c.eval(g * x64) - c.eval(0.0);
            shaped / (g * c.slope_at_zero())
        }
    };
    let m = mix.clamp(0.0, 1.0) as f64;
    ((1.0 - m) * x64 + m * wet) as f32
}
