//! Editable vocal expression curves — the per-lane data model (doc #154).
//!
//! A vocal lane carries an [`ExpressionCurves`] bundle of four curves —
//! **dynamics/energy, tension, breathiness, and pitch bend** — that let a
//! singer hand-shape the vocal's expression over time. Every curve is the
//! sum of two layers:
//!
//! 1. an auto-derived **baseline** (provenance) — evenly-spaced samples
//!    produced by the generator, kept around so an edit is always
//!    resettable, and
//! 2. a user **overlay** of editable breakpoints (time normalised to the
//!    clip/segment, plus a value). An empty overlay means the curve simply
//!    follows its baseline.
//!
//! [`ExpressionCurve::evaluate`] samples the effective curve at a
//! normalised time `t ∈ [0, 1]`; it is the single sampler shared by the UI
//! (sparklines, canvas) and the SVS segment builder (todo #334). A curve's
//! derived [`CurveStatus`] is `Auto` while untouched, `Edited` once the
//! user adds breakpoints, or `Na` when the active voicebank's acoustic
//! model doesn't accept it (see [`curve_supported`]).
//!
//! This is **data only** — no UI or render wiring lives here. The model is
//! `serde`-serializable so it can later persist with the project file
//! (todo #335); persistence into the project schema and the segment-build
//! feed are separate todos.

use serde::{Deserialize, Serialize};

use resonance_music_theory::VocalVoicebank;

use super::vocal_svs::{curve_supported, CurveKind};

/// One editable point in a curve's user overlay.
///
/// `t` is normalised to the clip/segment (`0.0` = start, `1.0` = end);
/// `value` is in the curve kind's [`CurveKind::value_range`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Breakpoint {
    /// Normalised time within the clip/segment, in `[0, 1]`.
    pub t: f32,
    /// Curve value at this point, in the owning kind's value range.
    pub value: f32,
}

impl Breakpoint {
    /// A breakpoint at normalised time `t` with `value`. Neither field is
    /// clamped here — the owning [`ExpressionCurve`] clamps on insertion so
    /// it can apply its kind's range.
    pub fn new(t: f32, value: f32) -> Self {
        Breakpoint { t, value }
    }
}

/// Derived display/render status of a single curve (doc #154).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CurveStatus {
    /// Untouched — the curve follows its auto-derived baseline.
    Auto,
    /// The user has shaped the curve with overlay breakpoints.
    Edited,
    /// The active voicebank's model doesn't accept this curve; it's a
    /// no-op on render and shown as `n/a` in the rail.
    Na,
}

/// A single expression curve: an auto-derived baseline plus an optional
/// user overlay of breakpoints layered on top.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpressionCurve {
    /// Which expression this curve shapes. Drives the value range used to
    /// clamp baseline and overlay values.
    kind: CurveKind,
    /// Auto-derived baseline — evenly-spaced samples over normalised time
    /// `[0, 1]`. Kept as provenance and as the reset target; never cleared
    /// by editing.
    baseline: Vec<f32>,
    /// User overlay, sorted ascending by `t`. Empty means the curve
    /// follows its baseline (the `Auto` status).
    overlay: Vec<Breakpoint>,
}

impl ExpressionCurve {
    /// A curve of `kind` with no baseline samples and no overlay. Evaluates
    /// to the neutral value for the kind until a baseline is seeded.
    pub fn new(kind: CurveKind) -> Self {
        ExpressionCurve {
            kind,
            baseline: Vec::new(),
            overlay: Vec::new(),
        }
    }

    /// A curve seeded from the generator's auto-derived `baseline` samples
    /// (evenly spaced over the clip). Out-of-range samples are clamped to
    /// the kind's [`CurveKind::value_range`]. The overlay starts empty, so
    /// the curve's status is [`CurveStatus::Auto`].
    pub fn from_baseline(kind: CurveKind, baseline: Vec<f32>) -> Self {
        let (lo, hi) = kind.value_range();
        let baseline = baseline.into_iter().map(|v| v.clamp(lo, hi)).collect();
        ExpressionCurve {
            kind,
            baseline,
            overlay: Vec::new(),
        }
    }

    /// The curve's kind.
    pub fn kind(&self) -> CurveKind {
        self.kind
    }

    /// The auto-derived baseline samples (evenly spaced over `[0, 1]`).
    pub fn baseline(&self) -> &[f32] {
        &self.baseline
    }

    /// The user overlay breakpoints, sorted ascending by `t`.
    pub fn overlay(&self) -> &[Breakpoint] {
        &self.overlay
    }

    /// Replace the baseline with freshly auto-derived `samples` (clamped to
    /// range). The overlay is left untouched, so a user edit survives a
    /// regenerate that only refreshes provenance.
    pub fn set_baseline(&mut self, samples: Vec<f32>) {
        let (lo, hi) = self.kind.value_range();
        self.baseline = samples.into_iter().map(|v| v.clamp(lo, hi)).collect();
    }

    /// True once the user has shaped this curve (its overlay is non-empty),
    /// i.e. the effective curve no longer follows the baseline.
    pub fn is_edited(&self) -> bool {
        !self.overlay.is_empty()
    }

    /// Drop the user overlay so the curve follows its auto-derived baseline
    /// again. The baseline (provenance) is preserved.
    pub fn reset_to_baseline(&mut self) {
        self.overlay.clear();
    }

    /// Replace the entire overlay. Each point's `t` is clamped to `[0, 1]`
    /// and its `value` to the kind's range; the result is sorted by `t`.
    /// Passing an empty vec is equivalent to [`Self::reset_to_baseline`].
    pub fn set_overlay(&mut self, points: Vec<Breakpoint>) {
        let (lo, hi) = self.kind.value_range();
        self.overlay = points
            .into_iter()
            .map(|p| Breakpoint {
                t: p.t.clamp(0.0, 1.0),
                value: p.value.clamp(lo, hi),
            })
            .collect();
        self.overlay
            .sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    }

    /// Add one breakpoint to the overlay (clamped to range/time), keeping
    /// the overlay sorted by `t`. Turns an `Auto` curve into `Edited`.
    pub fn add_breakpoint(&mut self, t: f32, value: f32) {
        let (lo, hi) = self.kind.value_range();
        let bp = Breakpoint {
            t: t.clamp(0.0, 1.0),
            value: value.clamp(lo, hi),
        };
        let idx = self
            .overlay
            .partition_point(|p| p.t <= bp.t);
        self.overlay.insert(idx, bp);
    }

    /// Effective curve value at normalised time `t ∈ [0, 1]` (clamped).
    ///
    /// When the overlay is non-empty the value is a piecewise-linear
    /// interpolation of the breakpoints (held flat past the first/last
    /// point); otherwise the baseline is linearly sampled. With neither a
    /// baseline nor an overlay the neutral value for the kind is returned.
    /// This is the shared sampler for both the UI and the segment builder.
    pub fn evaluate(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if !self.overlay.is_empty() {
            sample_breakpoints(&self.overlay, t)
        } else if !self.baseline.is_empty() {
            sample_uniform(&self.baseline, t)
        } else {
            neutral_value(self.kind)
        }
    }

    /// Derived [`CurveStatus`] given whether the active voicebank accepts
    /// this curve. Unsupported curves are `Na` regardless of edits.
    pub fn status(&self, supported: bool) -> CurveStatus {
        if !supported {
            CurveStatus::Na
        } else if self.is_edited() {
            CurveStatus::Edited
        } else {
            CurveStatus::Auto
        }
    }
}

/// The four editable expression curves for one vocal lane/clip (doc #154).
///
/// Construct from the generator's auto-derived baselines with
/// [`ExpressionCurves::from_baselines`]; [`ExpressionCurves::default`]
/// gives an empty bundle (flat curves, no overlay) for a lane that hasn't
/// been generated yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpressionCurves {
    /// Dynamics / energy envelope (loudness over time).
    pub dynamics: ExpressionCurve,
    /// Vocal tension (relaxed ↔ belted).
    pub tension: ExpressionCurve,
    /// Breathiness (added air in the delivery).
    pub breathiness: ExpressionCurve,
    /// Pitch bend — f0 offset / portamento in cents.
    pub pitch_bend: ExpressionCurve,
}

impl Default for ExpressionCurves {
    fn default() -> Self {
        ExpressionCurves {
            dynamics: ExpressionCurve::new(CurveKind::Dynamics),
            tension: ExpressionCurve::new(CurveKind::Tension),
            breathiness: ExpressionCurve::new(CurveKind::Breathiness),
            pitch_bend: ExpressionCurve::new(CurveKind::PitchBend),
        }
    }
}

impl ExpressionCurves {
    /// Seed every curve from its generator-derived baseline samples. Each
    /// vec is evenly spaced over the clip; pass an empty vec for a curve
    /// with no derivation yet (it evaluates to the kind's neutral value).
    pub fn from_baselines(
        dynamics: Vec<f32>,
        tension: Vec<f32>,
        breathiness: Vec<f32>,
        pitch_bend: Vec<f32>,
    ) -> Self {
        ExpressionCurves {
            dynamics: ExpressionCurve::from_baseline(CurveKind::Dynamics, dynamics),
            tension: ExpressionCurve::from_baseline(CurveKind::Tension, tension),
            breathiness: ExpressionCurve::from_baseline(CurveKind::Breathiness, breathiness),
            pitch_bend: ExpressionCurve::from_baseline(CurveKind::PitchBend, pitch_bend),
        }
    }

    /// Shared reference to the curve of `kind`.
    pub fn curve(&self, kind: CurveKind) -> &ExpressionCurve {
        match kind {
            CurveKind::Dynamics => &self.dynamics,
            CurveKind::Tension => &self.tension,
            CurveKind::Breathiness => &self.breathiness,
            CurveKind::PitchBend => &self.pitch_bend,
        }
    }

    /// Mutable reference to the curve of `kind`.
    pub fn curve_mut(&mut self, kind: CurveKind) -> &mut ExpressionCurve {
        match kind {
            CurveKind::Dynamics => &mut self.dynamics,
            CurveKind::Tension => &mut self.tension,
            CurveKind::Breathiness => &mut self.breathiness,
            CurveKind::PitchBend => &mut self.pitch_bend,
        }
    }

    /// Evaluate the curve of `kind` at normalised time `t`.
    pub fn evaluate(&self, kind: CurveKind, t: f32) -> f32 {
        self.curve(kind).evaluate(t)
    }

    /// Whether the user has shaped the curve of `kind`.
    pub fn is_edited(&self, kind: CurveKind) -> bool {
        self.curve(kind).is_edited()
    }

    /// Whether any of the four curves carries a user overlay.
    pub fn any_edited(&self) -> bool {
        CurveKind::ALL.iter().any(|&k| self.is_edited(k))
    }

    /// Reset the curve of `kind` back to its baseline (drops the overlay).
    pub fn reset(&mut self, kind: CurveKind) {
        self.curve_mut(kind).reset_to_baseline();
    }

    /// Reset every curve back to its baseline.
    pub fn reset_all(&mut self) {
        for &k in &CurveKind::ALL {
            self.reset(k);
        }
    }

    /// Derived [`CurveStatus`] of the curve of `kind` for `voicebank`,
    /// resolving voicebank support via [`curve_supported`].
    pub fn status(&self, kind: CurveKind, voicebank: VocalVoicebank) -> CurveStatus {
        self.curve(kind)
            .status(curve_supported(voicebank, kind))
    }
}

/// Neutral resting value for a kind with neither baseline nor overlay.
/// The `0..=1` envelopes rest at `0.0`; pitch bend rests at `0` cents.
/// Every kind's neutral happens to be the low end of its range except
/// pitch bend (whose range is symmetric about `0`), so resolve it
/// explicitly per kind rather than from the range bounds.
fn neutral_value(kind: CurveKind) -> f32 {
    match kind {
        CurveKind::Dynamics | CurveKind::Tension | CurveKind::Breathiness => 0.0,
        CurveKind::PitchBend => 0.0,
    }
}

/// Linearly sample an array of evenly-spaced `samples` at normalised time
/// `t ∈ [0, 1]`. `samples[0]` sits at `t = 0` and `samples[len-1]` at
/// `t = 1`.
fn sample_uniform(samples: &[f32], t: f32) -> f32 {
    match samples.len() {
        0 => 0.0,
        1 => samples[0],
        n => {
            let pos = t.clamp(0.0, 1.0) * (n - 1) as f32;
            let i = pos.floor() as usize;
            if i >= n - 1 {
                return samples[n - 1];
            }
            let frac = pos - i as f32;
            samples[i] * (1.0 - frac) + samples[i + 1] * frac
        }
    }
}

/// Piecewise-linear interpolation of overlay `points` (sorted ascending by
/// `t`) at normalised time `t`. The value is held flat before the first
/// and after the last breakpoint.
fn sample_breakpoints(points: &[Breakpoint], t: f32) -> f32 {
    match points {
        [] => 0.0,
        [only] => only.value,
        _ => {
            let first = &points[0];
            let last = &points[points.len() - 1];
            if t <= first.t {
                return first.value;
            }
            if t >= last.t {
                return last.value;
            }
            for w in points.windows(2) {
                let (a, b) = (&w[0], &w[1]);
                if t >= a.t && t <= b.t {
                    let span = b.t - a.t;
                    if span <= 0.0 {
                        return b.value;
                    }
                    let frac = (t - a.t) / span;
                    return a.value * (1.0 - frac) + b.value * frac;
                }
            }
            last.value
        }
    }
}
