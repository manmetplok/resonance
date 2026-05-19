//! Per-frame tension curve. Maps to the `tension` ONNX input on
//! voicebanks that expose it (Lilia, Meiji). Range `[-1, +1]`: `-1` =
//! relaxed / breathy delivery, `0` = neutral, `+1` = compressed /
//! belted. TIGER doesn't accept tension (the pipeline's
//! `flags.tension` will be false for that voicebank, so the curve is
//! ignored).
//!
//! Two modulators add per-frame movement to the slider baseline:
//!   - **Velocity**: strong-beat syllables (higher per-note velocity
//!     from `derive_vocal`) push tension up; weak ones push down.
//!   - **Contour**: notes near the top of the section's pitch range
//!     push tension up (singers belt at the top of their range); notes
//!     near the bottom push down.
//!
//! Each modulator's strength is its own slider in `[0, 1]` so the user
//! can dial in either, both, or neither.

use resonance_music_theory::VocalParams;
use resonance_svs::ds::SampleCurve;

use super::super::paths::voicebank_supports_tension;
use super::f0::{F0Curve, F0_TIMESTEP};

/// Build the tension `SampleCurve`. Returns `SampleCurve::default()`
/// when the voicebank doesn't accept a tension input (so the pipeline
/// can ignore the curve cheaply).
pub(super) fn build_tension_curve(curve: &F0Curve, params: &VocalParams) -> SampleCurve {
    if !voicebank_supports_tension(params.voicebank) {
        return SampleCurve::default();
    }
    let base = params.tension.clamp(-1.0, 1.0) as f64;
    let vel_amount = params.tension_velocity_amount.clamp(0.0, 1.0) as f64;
    let contour_amount = params.tension_contour_amount.clamp(0.0, 1.0) as f64;
    // Section pitch range, used to normalise the contour
    // contribution. Use the f0 sample range (excluding silence fill)
    // so the modulation is per-section rather than global.
    let (mut min_hz, mut max_hz) = (f64::INFINITY, 0.0_f64);
    for (i, &v) in curve.samples.iter().enumerate() {
        if curve.frame_note_total_sec.get(i).copied().unwrap_or(0.0) > 0.0 && v > 0.0 {
            if v < min_hz {
                min_hz = v;
            }
            if v > max_hz {
                max_hz = v;
            }
        }
    }
    let mid_hz = (min_hz + max_hz) * 0.5;
    let half_range_hz = ((max_hz - min_hz) * 0.5).max(1.0);
    let curve_len = curve.samples.len();
    let mut samples = Vec::with_capacity(curve_len);
    for (i, &pitch) in curve.samples.iter().enumerate().take(curve_len) {
        // Velocity modulation: derive_vocal's neutral velocity is
        // ~0.78 with strong beats around 0.86. Map to roughly
        // [-1, +1] around neutral, then scale by amount and
        // contribute up to ±0.5.
        let vel = curve.frame_velocity.get(i).copied().unwrap_or(0.0) as f64;
        let vel_mod = if vel > 0.0 {
            ((vel - 0.78) / 0.22).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        // Pitch contour modulation: position within section's f0
        // range, mapped to [-1, +1]. Silence frames contribute 0.
        let in_voiced =
            curve.frame_note_total_sec.get(i).copied().unwrap_or(0.0) > 0.0 && pitch > 0.0;
        let pitch_mod = if in_voiced {
            ((pitch - mid_hz) / half_range_hz).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let t = (base + vel_amount * vel_mod * 0.5 + contour_amount * pitch_mod * 0.5)
            .clamp(-1.0, 1.0);
        samples.push(t);
    }
    SampleCurve {
        samples,
        timestep: F0_TIMESTEP,
    }
}
