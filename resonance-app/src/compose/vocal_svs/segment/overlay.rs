//! Layering the user's editable expression overlay (doc #154, epic #17)
//! onto the per-frame curves the SVS segment builder emits — todo #334.
//!
//! Each of the four expression curves is the sum of an auto-derived
//! **baseline** (provenance) and an optional user **overlay** of
//! breakpoints (see [`crate::compose::expression`]). This module turns an
//! [`ExpressionCurve`] into the per-frame samples the pipeline accepts,
//! sampled at the **same `F0_TIMESTEP` grid as the f0 curve** so every
//! curve lines up frame-for-frame:
//!
//! - Dynamics  → [`DsSegment::energy`](resonance_svs::ds::DsSegment::energy)
//! - Breathiness → `DsSegment::breathiness`
//! - Tension → `DsSegment::tension`
//! - Pitch bend → an additive cents offset folded into the f0 curve
//!   (applied in [`super::f0`], after portamento + vibrato).
//!
//! ## When the overlay applies
//! The override only kicks in when the curve carries actual overlay
//! breakpoints. A curve with an empty overlay (the `Auto` state, or a
//! depth/smoothing-only tweak that has nothing to reshape) is left to the
//! builder's auto-derived path, so the rendered result is **byte-for-byte
//! identical** to a pre-#334 render. `depth` and `smoothing` modulate the
//! overlay's deviation from its baseline, so they have no effect without
//! breakpoints to act on.
//!
//! ## Voicebank support
//! [`curve_supported`] (todo #333) is the single source of truth for
//! whether a voicebank's acoustic model accepts a curve. An unsupported
//! curve is a clean no-op here — the corresponding `DsSegment` field is
//! left at its default — never an error. Pitch bend and dynamics are
//! universally supported; tension and breathiness are TIGER-unsupported.
//!
//! ## Value mapping
//! The dynamics / tension / breathiness overlays are drawn on a `0..=1`
//! axis (doc #154) but the acoustic model's per-frame inputs are *signed*,
//! centred on `0.0` (neutral). We map linearly so the envelope midpoint
//! (`0.5`) is model-neutral and the extremes hit `±1`, matching the
//! established tension convention ([`super::tension`] emits `[-1, +1]`).
//! Pitch bend is already in cents, fed straight through.

use resonance_music_theory::VocalVoicebank;
use resonance_svs::ds::SampleCurve;

use crate::compose::expression::ExpressionCurve;

use super::super::paths::curve_supported;
use super::f0::F0_TIMESTEP;

/// Per-frame envelope (Dynamics → energy, Tension → tension, Breathiness →
/// breathiness) sampled from the user overlay across `n_frames`, in the
/// acoustic model's signed input units.
///
/// Returns `None` — leaving the caller on its auto-derived path — when the
/// curve has no overlay breakpoints, the active `voicebank` doesn't accept
/// the curve, or there are no frames to sample.
///
/// `window` maps the segment's local frame span onto the clip-normalised
/// time the overlay breakpoints live in: frame `i` samples the overlay at
/// `window.0 + (i / (n-1)) * (window.1 - window.0)`. A whole-clip segment
/// passes `(0.0, 1.0)`; a sub-clip render unit passes its clip-relative
/// fraction so the overlay isn't re-stretched onto each unit.
pub(super) fn overlay_envelope(
    curve: &ExpressionCurve,
    voicebank: VocalVoicebank,
    n_frames: usize,
    window: (f32, f32),
) -> Option<SampleCurve> {
    if curve.overlay().is_empty()
        || n_frames == 0
        || !curve_supported(voicebank, curve.kind())
    {
        return None;
    }
    let samples = sample_curve_frames(curve, n_frames, window)
        .into_iter()
        .map(envelope_to_model)
        .collect();
    Some(SampleCurve {
        samples,
        timestep: F0_TIMESTEP,
    })
}

/// Per-frame additive pitch offset in **cents** from the pitch-bend
/// overlay across `n_frames`, or `None` when the overlay is empty (f0 left
/// untouched). Pitch bend is a pre-synthesis f0 edit accepted by every
/// voicebank, so there's no support gate. `window` maps frames to
/// clip-normalised time exactly as in [`overlay_envelope`].
pub(super) fn overlay_pitch_cents(
    curve: &ExpressionCurve,
    n_frames: usize,
    window: (f32, f32),
) -> Option<Vec<f64>> {
    if curve.overlay().is_empty() || n_frames == 0 {
        return None;
    }
    Some(
        sample_curve_frames(curve, n_frames, window)
            .into_iter()
            .map(|v| v as f64)
            .collect(),
    )
}

/// Sample the effective curve (overlay layered over baseline, scaled by
/// `depth`, then `smoothing`-windowed) across `n` frames in the curve's
/// own value range. Only called once an overlay is known present.
fn sample_curve_frames(curve: &ExpressionCurve, n: usize, window: (f32, f32)) -> Vec<f32> {
    let depth = curve.depth();
    let baseline = curve.baseline();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let frac = if n <= 1 {
            0.0
        } else {
            i as f32 / (n - 1) as f32
        };
        let t = window.0 + frac * (window.1 - window.0);
        // `depth` scales how far the shaped overlay departs from the
        // auto-derived baseline: depth 0 applies the overlay as drawn,
        // -1 collapses it back to the baseline, +1 exaggerates it.
        let base_at = sample_uniform(baseline, t);
        let shaped = curve.evaluate(t);
        out.push(base_at + (1.0 + depth) * (shaped - base_at));
    }
    smooth(&mut out, curve.smoothing());
    let (lo, hi) = curve.kind().value_range();
    for v in &mut out {
        *v = v.clamp(lo, hi);
    }
    out
}

/// Linear sample of an evenly-spaced array at normalised `t ∈ [0, 1]`.
/// Mirrors the data model's own sampler so a baseline that round-trips
/// through here matches the UI. An empty array is the kind's neutral
/// (`0.0`), so an un-seeded baseline contributes no deviation.
fn sample_uniform(samples: &[f32], t: f32) -> f32 {
    match samples.len() {
        0 => 0.0,
        1 => samples[0],
        len => {
            let pos = t.clamp(0.0, 1.0) * (len - 1) as f32;
            let i = pos.floor() as usize;
            if i >= len - 1 {
                return samples[len - 1];
            }
            let frac = pos - i as f32;
            samples[i] * (1.0 - frac) + samples[i + 1] * frac
        }
    }
}

/// In-place centred moving-average smoothing with a window derived from
/// `smoothing_ms` at the f0 frame rate. A neutral / sub-frame window is a
/// no-op.
fn smooth(samples: &mut [f32], smoothing_ms: f32) {
    if smoothing_ms <= 0.0 || samples.len() < 3 {
        return;
    }
    let win = ((smoothing_ms as f64 / 1000.0) / F0_TIMESTEP).round() as usize;
    if win < 2 {
        return;
    }
    let half = (win / 2).max(1);
    let src = samples.to_vec();
    for (i, out) in samples.iter_mut().enumerate() {
        let lo = i.saturating_sub(half);
        let hi = (i + half + 1).min(src.len());
        let sum: f32 = src[lo..hi].iter().sum();
        *out = sum / (hi - lo) as f32;
    }
}

/// Map a normalised `0..=1` expression-envelope value to the acoustic
/// model's signed per-frame input (`-1..=+1`, `0` neutral). See the module
/// docs for the rationale.
fn envelope_to_model(v: f32) -> f64 {
    (v as f64) * 2.0 - 1.0
}
