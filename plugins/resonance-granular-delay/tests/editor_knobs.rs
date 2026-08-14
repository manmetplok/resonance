//! Knob-cell geometry and the plain↔unit mapping of the control strip
//! (ba todo #1266).
//!
//! The knobs themselves are now `wayland_plugin_gui::widgets`' themed
//! knob rather than a fork of it, so what is left to pin here is what
//! the granular editor actually decides: the two cell styles (they must
//! keep the exact footprint the strip layout and the division stepper
//! were built around) and the mapping between a parameter's plain range
//! and the unit space the shared widget speaks.

#![cfg(feature = "editor")]

use resonance_granular_delay::editor::widgets::{
    plain_of_unit, unit_of_plain, MACRO_KNOB_SIZE, MACRO_KNOB_STYLE, TEXTURE_KNOB_SIZE,
    TEXTURE_KNOB_STYLE,
};
use resonance_granular_delay::params::GranularDelayParams;

/// Cell footprints predate the shared-widget move; changing them
/// reflows the six strip groups at the 1320 px window width, and the
/// division stepper (which allocates the macro cell so the SYNC swap
/// causes no layout jump) with them.
#[test]
fn knob_cells_keep_their_footprint() {
    let macro_cell = MACRO_KNOB_STYLE.cell();
    assert_eq!(MACRO_KNOB_STYLE.diameter, MACRO_KNOB_SIZE);
    assert_eq!((macro_cell.x, macro_cell.y), (68.0, 86.0));

    let texture_cell = TEXTURE_KNOB_STYLE.cell();
    assert_eq!(TEXTURE_KNOB_STYLE.diameter, TEXTURE_KNOB_SIZE);
    assert_eq!((texture_cell.x, texture_cell.y), (50.0, 68.0));
}

/// Hierarchy by size, not chrome: the texture tier is the smaller dial
/// with the smaller type.
#[test]
fn texture_tier_is_smaller_than_the_macro_tier() {
    const {
        assert!(TEXTURE_KNOB_STYLE.diameter < MACRO_KNOB_STYLE.diameter);
        assert!(TEXTURE_KNOB_STYLE.value_font < MACRO_KNOB_STYLE.value_font);
        assert!(TEXTURE_KNOB_STYLE.label_font < MACRO_KNOB_STYLE.label_font);
    };
}

#[test]
fn plain_and_unit_round_trip() {
    // Bipolar pitch range.
    assert_eq!(unit_of_plain(-24.0, 24.0, 0.0), 0.5);
    assert_eq!(unit_of_plain(-24.0, 24.0, -24.0), 0.0);
    assert_eq!(unit_of_plain(-24.0, 24.0, 24.0), 1.0);
    assert_eq!(plain_of_unit(-24.0, 24.0, 0.5), 0.0);

    for unit in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
        let plain = plain_of_unit(20.0, 2000.0, unit);
        let back = unit_of_plain(20.0, 2000.0, plain);
        assert!((back - unit).abs() < 1e-6, "{unit} -> {plain} -> {back}");
    }
}

#[test]
fn plain_and_unit_clamp_out_of_range_values() {
    assert_eq!(unit_of_plain(0.0, 1.1, 5.0), 1.0);
    assert_eq!(unit_of_plain(0.0, 1.1, -5.0), 0.0);
    assert_eq!(plain_of_unit(0.0, 1.1, 2.0), 1.1);
    assert_eq!(plain_of_unit(0.0, 1.1, -2.0), 0.0);
    // A degenerate range must not divide by zero.
    assert!(unit_of_plain(1.0, 1.0, 1.0).is_finite());
}

/// The Feedback knob marks its over-unity zone from plain 1.0 (100 %)
/// up to the top of the range, which is what `feedback_knob` feeds the
/// shared widget as `warm_from`.
#[test]
fn feedback_over_unity_zone_starts_at_one_hundred_percent() {
    let params = GranularDelayParams::default();
    let p = params.param_at(4);
    assert_eq!(p.name(), "Feedback");

    let min = p.min_plain() as f32;
    let max = p.max_plain() as f32;
    let warm_from = unit_of_plain(min, max, 1.0);
    assert!(
        max > 1.0,
        "the feedback range no longer reaches over unity ({min}..{max})"
    );
    assert!(
        (plain_of_unit(min, max, warm_from) - 1.0).abs() < 1e-5,
        "warm zone starts at {} instead of plain 1.0",
        plain_of_unit(min, max, warm_from)
    );
    assert!(warm_from > 0.0 && warm_from < 1.0);
}
