//! Pure conversion helpers for tempo events.
//!
//! These functions operate on a `TempoPoint` slice rather than a full
//! `TempoMap`, so they can be reused by the bar-table builder and by
//! any caller that has the raw event list but not the precomputed
//! lookup structure.

use super::{TempoPoint};

/// Return the interpolated BPM at a fractional bar position.
/// Between events at different bars the BPM ramps linearly.
/// When multiple events share the same bar (step change) the last
/// value at that bar wins.
pub fn bpm_at_bar(bar: f64, tempo_points: &[TempoPoint]) -> f64 {
    if tempo_points.is_empty() {
        return 120.0;
    }
    let mut prev_bpm = tempo_points[0].bpm as f64;
    let mut prev_bar = tempo_points[0].bar as f64;
    let mut next: Option<&TempoPoint> = None;

    for e in tempo_points {
        if (e.bar as f64) <= bar {
            prev_bpm = e.bpm as f64;
            prev_bar = e.bar as f64;
        } else {
            next = Some(e);
            break;
        }
    }

    if let Some(ne) = next {
        if prev_bar < bar {
            let t = (bar - prev_bar) / (ne.bar as f64 - prev_bar);
            return prev_bpm + (ne.bpm as f64 - prev_bpm) * t;
        }
    }

    prev_bpm
}

/// Return the arrival BPM at a bar — the ramp target approaching this
/// bar from the left. When multiple events share the same bar (step
/// change), this returns the FIRST event's value; `bpm_at_bar` returns
/// the LAST (departure) value.
pub fn arrival_bpm_at_bar(bar: u32, tempo_points: &[TempoPoint]) -> f64 {
    if tempo_points.is_empty() {
        return 120.0;
    }
    // Return the first event at exactly this bar if one exists.
    for e in tempo_points {
        if e.bar == bar {
            return e.bpm as f64;
        }
        if e.bar > bar {
            break;
        }
    }
    // No event at this bar — arrival equals the interpolated value.
    bpm_at_bar(bar as f64, tempo_points)
}

/// Average BPM across a bar (departure at start, arrival at end) / 2.
/// Uses arrival BPM for the end so that step changes at bar boundaries
/// don't erase the ramp target.
pub fn avg_bpm_for_bar(bar: u32, tempo_points: &[TempoPoint]) -> f64 {
    let bpm_start = bpm_at_bar(bar as f64, tempo_points);
    let bpm_end = arrival_bpm_at_bar(bar + 1, tempo_points);
    (bpm_start + bpm_end) / 2.0
}

/// Map a tick fraction (0..1) within a bar to a sample fraction (0..1),
/// accounting for linear BPM interpolation within the bar.
/// When BPM ramps from `bpm_s` to `bpm_e`, the tick→sample mapping is
/// logarithmic: `g(f) = ln(1 + (r-1)*f) / ln(r)` where `r = bpm_e/bpm_s`.
pub fn tick_frac_to_sample_frac(tick_frac: f64, bpm_start: f64, bpm_end: f64) -> f64 {
    let r = bpm_end / bpm_start;
    if (r - 1.0).abs() < 1e-6 {
        return tick_frac;
    }
    (1.0 + (r - 1.0) * tick_frac).ln() / r.ln()
}

/// Map a sample fraction (0..1) within a bar to a tick fraction (0..1),
/// accounting for linear BPM interpolation within the bar.
/// Inverse of `tick_frac_to_sample_frac`.
pub fn sample_frac_to_tick_frac(sample_frac: f64, bpm_start: f64, bpm_end: f64) -> f64 {
    let r = bpm_end / bpm_start;
    if (r - 1.0).abs() < 1e-6 {
        return sample_frac;
    }
    (r.powf(sample_frac) - 1.0) / (r - 1.0)
}
