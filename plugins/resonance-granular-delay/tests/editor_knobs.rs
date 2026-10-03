//! Knob-cell geometry and the plain↔unit mapping of the control strip
//! (ba todo #1266).
//!
//! The knobs themselves are now `wayland_plugin_gui::widgets`' themed
//! knob rather than a fork of it, so what is left to pin here is what
//! the granular editor actually decides: the two cell styles (they must
//! keep the exact footprint the strip layout and the division stepper
//! were built around) and that each knob travels along its param's own
//! declared curve.

#![cfg(feature = "editor")]

use resonance_granular_delay::editor::widgets::{
    feedback_warm_from, float_at, MACRO_KNOB_SIZE, MACRO_KNOB_STYLE, TEXTURE_KNOB_SIZE,
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

/// Every knob travels along its param's own declared curve (code review
/// PUX-05): the strip used a linear plain-to-unit mapping, so the skewed
/// Filter put 20-500 Hz into about 5 px of arc. The binding is the
/// param's `normalized_value` / `set_normalized`, which is the whole
/// contract, so check a skewed param's midpoint is not the linear one.
#[test]
fn knobs_bind_through_the_declared_skew() {
    let params = GranularDelayParams::default();
    for i in [2usize, 7, 8, 15, 21] {
        let p = float_at(&params, i).expect("a knob param");
        p.set_normalized(0.5);
        let mid = p.value();
        let linear_mid = (p.range().min() + p.range().max()) * 0.5;
        assert!(
            (mid - linear_mid).abs() > 1.0,
            "{}: travel 0.5 lands on the linear midpoint {linear_mid}",
            resonance_plugin::Param::id(p)
        );
        assert!((p.normalized_value() - 0.5).abs() < 1e-4);
    }
}

/// The Feedback knob marks its over-unity zone from plain 1.0 (100 %)
/// up to the top of the range, which is what `feedback_knob` feeds the
/// shared widget as `warm_from`.
#[test]
fn feedback_over_unity_zone_starts_at_one_hundred_percent() {
    let params = GranularDelayParams::default();
    let p = &params.feedback;
    let warm_from = feedback_warm_from(&params);
    assert!(
        p.range().max() > 1.0,
        "the feedback range no longer reaches over unity"
    );
    assert!(
        (p.plain_at_normalized(warm_from) - 1.0).abs() < 1e-5,
        "warm zone starts at {} instead of plain 1.0",
        p.plain_at_normalized(warm_from)
    );
    assert!(warm_from > 0.0 && warm_from < 1.0);
}
