//! Pure-geometry coverage for the timeline automation-lane *input* layer
//! (todo #382 / A4, arch doc #162 §3). The live canvas hit-testing on
//! `TimelineCanvas` needs a fully-built struct, so the math it relies on is
//! factored into pure helpers — the value↔y inverse and the breakpoint
//! pick — which these tests pin without a canvas. The end-to-end pointer
//! flow is verified visually by the `iced_test` golden suite.

use iced::Point;
use resonance_app::view::timeline::automation::{
    nearest_breakpoint, value_from_y, value_to_y, BREAKPOINT_HIT_RADIUS,
};
use resonance_common::{Breakpoint, CurveKind};

/// 1 px per frame, so a breakpoint at frame `n` lands at x = `n`.
fn x_one_to_one(frames: u64) -> f32 {
    frames as f32
}

fn bp(time_frames: u64, value: f32) -> Breakpoint {
    Breakpoint::new(time_frames, value, CurveKind::Linear)
}

#[test]
fn value_from_y_inverts_value_to_y() {
    let (band_top, band_height) = (20.0_f32, 80.0_f32);
    for v in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
        let y = value_to_y(v, band_top, band_height);
        let back = value_from_y(y, band_top, band_height);
        assert!((back - v).abs() < 1e-6, "round-trip {v} -> {back}");
    }
}

#[test]
fn value_from_y_clamps_outside_the_band() {
    let (band_top, band_height) = (20.0_f32, 80.0_f32);
    // Above the band reads as the max value, below as the min.
    assert_eq!(value_from_y(band_top - 50.0, band_top, band_height), 1.0);
    assert_eq!(
        value_from_y(band_top + band_height + 50.0, band_top, band_height),
        0.0
    );
}

#[test]
fn value_from_y_zero_height_band_is_safe() {
    assert_eq!(value_from_y(42.0, 10.0, 0.0), 0.0);
}

#[test]
fn nearest_breakpoint_picks_the_dot_under_the_pointer() {
    let pts = [bp(0, 0.0), bp(100, 0.5), bp(200, 1.0)];
    let (band_top, band_height) = (0.0_f32, 100.0_f32);
    // Right on the middle dot: frame 100 -> x 100, value 0.5 -> y 50.
    let hit = nearest_breakpoint(
        &pts,
        Point::new(100.0, 50.0),
        band_top,
        band_height,
        x_one_to_one,
        BREAKPOINT_HIT_RADIUS,
    );
    assert_eq!(hit, Some(1));
}

#[test]
fn nearest_breakpoint_misses_when_outside_the_radius() {
    let pts = [bp(100, 0.5)];
    let (band_top, band_height) = (0.0_f32, 100.0_f32);
    // 20 px away from the only dot — well outside the 7 px pick radius.
    let hit = nearest_breakpoint(
        &pts,
        Point::new(120.0, 50.0),
        band_top,
        band_height,
        x_one_to_one,
        BREAKPOINT_HIT_RADIUS,
    );
    assert_eq!(hit, None);
}

#[test]
fn nearest_breakpoint_breaks_ties_toward_the_closest() {
    // Two dots within the pick radius of the pointer; the nearer one wins.
    let pts = [bp(98, 0.5), bp(103, 0.5)];
    let (band_top, band_height) = (0.0_f32, 100.0_f32);
    let y = value_to_y(0.5, band_top, band_height);
    // Pointer at x=102 is 4 px from dot 0 and 1 px from dot 1.
    let hit = nearest_breakpoint(
        &pts,
        Point::new(102.0, y),
        band_top,
        band_height,
        x_one_to_one,
        BREAKPOINT_HIT_RADIUS,
    );
    assert_eq!(hit, Some(1));
}

#[test]
fn nearest_breakpoint_empty_lane_is_a_miss() {
    let hit = nearest_breakpoint(
        &[],
        Point::new(0.0, 0.0),
        0.0,
        100.0,
        x_one_to_one,
        BREAKPOINT_HIT_RADIUS,
    );
    assert_eq!(hit, None);
}
