//! The transfer plot's dB window against the threshold parameter that
//! defines it (ba todo #1346).
//!
//! `curve.rs` used to declare `DB_MIN = -60.0` / `DB_MAX = 0.0`, which
//! happened to be exactly the threshold's declared range. That is the
//! same defect class as finding F4 — a fact restated at a call site that
//! `params.rs` already owns — with a quieter failure mode: the plot
//! exists to show where the threshold sits, so a range change would not
//! have broken a build, it would have slid the threshold indicator off
//! the plot and clamped it against the frame.
//!
//! These tests pin the relationship rather than the numbers: whatever
//! range `params.rs` declares, the window covers it.

#![cfg(feature = "editor")]

use resonance_compressor::editor::curve::DbAxis;
use resonance_compressor::params::CompressorParams;
use resonance_plugin::{FloatParam, FloatRange};

/// The renderer's own source, compiled in, so the guard below can never
/// drift from the file the editor actually builds.
const CURVE_SRC: &str = include_str!("../src/editor/curve.rs");

fn axis() -> (CompressorParams, DbAxis) {
    let params = CompressorParams::default();
    let axis = DbAxis::from_threshold(&params.threshold);
    (params, axis)
}

// ---------------------------------------------------------------------------
// Coverage: every threshold the user can dial in is on the plot
// ---------------------------------------------------------------------------

#[test]
fn the_window_covers_the_thresholds_whole_declared_range() {
    let (params, axis) = axis();
    let range = params.threshold.range();

    assert!(
        axis.min() <= range.min(),
        "the plot starts at {} but the threshold reaches down to {}",
        axis.min(),
        range.min()
    );
    assert!(
        axis.max() >= range.max(),
        "the plot stops at {} but the threshold reaches up to {}",
        axis.max(),
        range.max()
    );
}

#[test]
fn no_reachable_threshold_clamps_against_the_frame() {
    let (params, axis) = axis();
    let range = params.threshold.range();

    // Sweep the parameter's own travel, which is the set of values a
    // user, a preset or the host can actually produce.
    let mut previous = f32::NEG_INFINITY;
    for step in 0..=200 {
        let t = step as f32 / 200.0;
        let threshold = params.threshold.plain_at_normalized(t);
        let fraction = axis.fraction(threshold);

        assert!(
            (0.0..=1.0).contains(&fraction),
            "threshold {threshold} maps to {fraction} of the plot"
        );
        assert!(
            fraction >= previous,
            "the indicator moved backwards at t = {t} ({threshold} dB)"
        );
        // Clamping shows up as two distinct thresholds sharing a
        // position. Only the extremes may sit exactly on an edge.
        if threshold > range.min() && threshold < range.max() {
            assert!(
                fraction > 0.0 && fraction < 1.0,
                "threshold {threshold} is inside the declared range but pinned to the frame"
            );
        }
        previous = fraction;
    }

    // And the extremes land on the ends, not somewhere short of them.
    assert_eq!(axis.fraction(range.min()), 0.0);
    assert_eq!(axis.fraction(range.max()), 1.0);
}

#[test]
fn the_window_follows_the_declaration_rather_than_a_copy_of_it() {
    // A threshold declared over a different span must move the plot with
    // it; this is the drift the old constants could not have survived.
    let moved = FloatParam::new(
        "threshold",
        "Threshold",
        -30.0,
        FloatRange::Linear {
            min: -90.0,
            max: 6.0,
        },
    );
    let axis = DbAxis::from_threshold(&moved);
    assert!(axis.min() <= -90.0 && axis.max() >= 6.0);
    assert_eq!(axis.fraction(-90.0), 0.0);
    assert_eq!(axis.fraction(6.0), 1.0);
    assert!((axis.fraction(-42.0) - 0.5).abs() < 1e-6);
}

#[test]
fn the_reference_grid_spans_the_same_window() {
    let (_, axis) = axis();
    let levels: Vec<f32> = axis.grid_levels().collect();

    assert!(levels.len() >= 2, "a grid needs both ends");
    assert_eq!(*levels.first().unwrap(), axis.min());
    assert!(
        (levels.last().unwrap() - axis.max()).abs() < 1e-4,
        "the top grid line is at {} for a window ending at {}",
        levels.last().unwrap(),
        axis.max()
    );
    // Evenly spaced, so the grid stays readable at any declared range.
    let step = (axis.max() - axis.min()) / (levels.len() - 1) as f32;
    for (i, level) in levels.iter().enumerate() {
        assert!((level - (axis.min() + i as f32 * step)).abs() < 1e-4);
    }
}

// ---------------------------------------------------------------------------
// Source guard: the renderer states no bounds of its own
// ---------------------------------------------------------------------------

#[test]
fn the_renderer_declares_no_axis_bounds_of_its_own() {
    let params = CompressorParams::default();
    let declared_min = params.threshold.range().min();

    // Skip comments, which talk *about* the constants this file no
    // longer has, exactly as the control-strip guard does.
    let body: String = CURVE_SRC
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !body.contains("DB_MIN") && !body.contains("DB_MAX"),
        "the plot window belongs to params.rs; curve.rs must not keep its own constants"
    );
    for spelling in [format!("{declared_min:.0}"), format!("{declared_min:.1}")] {
        assert!(
            !body.contains(&spelling),
            "`{spelling}` is the threshold's declared minimum showing up as a literal in curve.rs"
        );
    }
    assert!(
        body.contains("threshold.range()"),
        "the window must be read off the threshold parameter"
    );
}
