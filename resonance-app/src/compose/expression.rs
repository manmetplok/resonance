//! Vocal expression curves — the app side of the curves a vocal lane
//! carries in its config (`VocalParams::curves`,
//! vocal-mcp-control-and-quality.md §10).
//!
//! A lane has four curves — **Dynamics, Tension, Breathiness and Pitch
//! bend** — each a list of section-relative points: `beat` in quarter
//! notes from the section start, `value` the normalized `0..=1` lane
//! value. They are edited as `Vocal:` automation sub-rows on the Arrange
//! view (every placement of the section shows, and edits, the same curve;
//! see [`super::expression_lanes`]) and through `automation.*`, saved with
//! the project and undone with it.
//!
//! This module is the shared vocabulary: the real units a normalized value
//! means ([`real_value`]), the sampler the segment builder reads
//! ([`sample_at_beat`]), which curves a bank honours ([`curve_audible`]),
//! and the point edits the update handlers apply. An empty curve leaves the
//! render on its own derivation, so a lane nobody shaped renders exactly as
//! before the curves existed.

use resonance_music_theory::{VocalCurvePoint, VocalCurves};

use super::vocal_svs::{BankProfile, CurveKind};

/// The normalized value every curve treats as "no change": unity gain,
/// the model-neutral tension / breathiness, zero cents of bend.
pub const NEUTRAL: f32 = 0.5;

/// Two beats closer than this are the same position (beats are stored as
/// whole ticks over the tick rate, so real distinct positions are at least
/// a tick — 1/480 beat — apart).
const BEAT_EPSILON: f64 = 1e-6;

/// The curve of `kind` in `curves`.
pub fn curve(curves: &VocalCurves, kind: CurveKind) -> &[VocalCurvePoint] {
    curves.get(kind.index()).map(Vec::as_slice).unwrap_or(&[])
}

/// Mutable [`curve`].
pub fn curve_mut(curves: &mut VocalCurves, kind: CurveKind) -> &mut Vec<VocalCurvePoint> {
    curves
        .get_mut(kind.index())
        .expect("VocalCurves holds one curve per CurveKind")
}

/// The real value a normalized `norm` stands for on `kind`: the `0..=1`
/// envelopes as themselves, pitch bend in cents (`-50..=50`).
pub fn real_value(kind: CurveKind, norm: f32) -> f32 {
    let (lo, hi) = kind.value_range();
    lo + norm.clamp(0.0, 1.0) * (hi - lo)
}

/// Inverse of [`real_value`], clamped to `0..=1`.
pub fn normalized_value(kind: CurveKind, real: f32) -> f32 {
    let (lo, hi) = kind.value_range();
    ((real - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// Whether `bank` renders `kind`. Pitch bend is an f0 edit every bank
/// takes; Dynamics drives the bank's energy input where it has one and a
/// post-render gain everywhere else, so it is audible on every bank too.
/// Tension and Breathiness need the matching acoustic-model input.
pub fn curve_audible(kind: CurveKind, bank: &BankProfile) -> bool {
    match kind {
        CurveKind::PitchBend | CurveKind::Dynamics => true,
        CurveKind::Tension | CurveKind::Breathiness => bank.curve_supported(kind),
    }
}

/// The normalized value of `points` at `beat`: linear between points
/// (held after a `stepped` one), flat before the first and after the last.
/// `None` for an empty curve.
pub fn sample_at_beat(points: &[VocalCurvePoint], beat: f64) -> Option<f32> {
    let first = points.first()?;
    let last = points.last()?;
    if beat <= first.beat {
        return Some(first.value);
    }
    if beat >= last.beat {
        return Some(last.value);
    }
    let right = points.partition_point(|p| p.beat <= beat);
    let (a, b) = (&points[right - 1], &points[right]);
    if a.stepped {
        return Some(a.value);
    }
    let span = b.beat - a.beat;
    if span <= 0.0 {
        return Some(b.value);
    }
    let t = ((beat - a.beat) / span) as f32;
    Some(a.value + (b.value - a.value) * t)
}

/// Insert `point`, keeping the curve sorted; a point already at the same
/// beat is replaced. Returns the index it landed at.
pub fn upsert_point(points: &mut Vec<VocalCurvePoint>, point: VocalCurvePoint) -> usize {
    let point = VocalCurvePoint {
        beat: point.beat.max(0.0),
        value: point.value.clamp(0.0, 1.0),
        stepped: point.stepped,
    };
    if let Some(i) = points
        .iter()
        .position(|p| (p.beat - point.beat).abs() < BEAT_EPSILON)
    {
        points[i] = point;
        return i;
    }
    let idx = points.partition_point(|p| p.beat < point.beat);
    points.insert(idx, point);
    idx
}

/// Move the point at `index` to (`beat`, `value`). The beat is clamped
/// between its neighbours (and to `0..=max_beat`), so a drag never
/// reorders the curve and the index stays put. Out of range: no-op.
pub fn move_point(
    points: &mut [VocalCurvePoint],
    index: usize,
    beat: f64,
    value: f32,
    max_beat: f64,
) {
    if index >= points.len() {
        return;
    }
    let lo = if index > 0 { points[index - 1].beat } else { 0.0 };
    let hi = points
        .get(index + 1)
        .map_or(max_beat.max(lo), |p| p.beat);
    let p = &mut points[index];
    p.beat = beat.clamp(lo, hi.max(lo));
    p.value = value.clamp(0.0, 1.0);
}

/// Remove the point at `index` (out of range: no-op).
pub fn remove_point(points: &mut Vec<VocalCurvePoint>, index: usize) {
    if index < points.len() {
        points.remove(index);
    }
}

/// Set whether the point at `index` holds its value to the next point.
pub fn set_stepped(points: &mut [VocalCurvePoint], index: usize, stepped: bool) {
    if let Some(p) = points.get_mut(index) {
        p.stepped = stepped;
    }
}
