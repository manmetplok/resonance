//! The platform-wide knob drag feel (ba todo #1266).
//!
//! Both knob families — the classic range-mapped `knob` and the themed
//! `knob_themed` one the lavender editors use — resolve a vertical drag
//! through `knob_drag_unit`. These tests pin the numbers, because the
//! whole point of the shared helper is that a plugin cannot quietly
//! grow its own drag feel again: the granular delay used to answer the
//! same gesture at 0.008 per pixel (0.002 with Shift) while everything
//! else ran at 0.005.

use plugin_gui_core::widgets::{
    knob_drag_unit, KnobStyle, KNOB_DRAG_SPEED, KNOB_DRAG_SPEED_FINE,
};

/// Egui reports downward pointer motion as a positive `drag_delta().y`,
/// and dragging down must lower the value.
#[test]
fn dragging_down_lowers_the_value() {
    assert!(knob_drag_unit(0.5, 10.0, false) < 0.5);
    assert!(knob_drag_unit(0.5, -10.0, false) > 0.5);
}

#[test]
fn full_scale_sweep_is_two_hundred_pixels() {
    assert_eq!(KNOB_DRAG_SPEED, 0.005);
    // 200 px of upward drag covers exactly 0..1.
    let unit = knob_drag_unit(0.0, -200.0, false);
    assert!((unit - 1.0).abs() < 1e-6, "expected full sweep, got {unit}");
    // Half that, half the range.
    let half = knob_drag_unit(0.0, -100.0, false);
    assert!((half - 0.5).abs() < 1e-6, "expected half sweep, got {half}");
}

#[test]
fn shift_is_the_finer_of_the_two_speeds() {
    assert_eq!(KNOB_DRAG_SPEED_FINE, 0.001);
    // The Shift speed must be the slower of the two — and by enough to
    // feel like a different gesture. Merely slower is not enough: this
    // was briefly 0.004 against a 0.005 normal speed, a 20% difference
    // no one can perceive while dragging. Require at least 3x finer so
    // a future tweak cannot quietly reduce Shift to a no-op again.
    const { assert!(KNOB_DRAG_SPEED_FINE < KNOB_DRAG_SPEED) };
    const { assert!(KNOB_DRAG_SPEED >= KNOB_DRAG_SPEED_FINE * 3.0) };
    let coarse = knob_drag_unit(0.5, -20.0, false);
    let fine = knob_drag_unit(0.5, -20.0, true);
    assert!(
        fine < coarse,
        "Shift moved the value {fine} at least as far as the plain drag {coarse}"
    );
}

#[test]
fn drag_clamps_to_unit_range() {
    assert_eq!(knob_drag_unit(0.9, -1000.0, false), 1.0);
    assert_eq!(knob_drag_unit(0.1, 1000.0, false), 0.0);
    assert_eq!(knob_drag_unit(0.5, 0.0, false), 0.5);
}

/// The default cell is the 52 px lavender knob the drums editor lays
/// out with; a style change here reflows every editor using it.
#[test]
fn lavender_style_cell_is_unchanged() {
    let cell = KnobStyle::LAVENDER.cell();
    assert_eq!((cell.x, cell.y), (60.0, 84.0));
    assert_eq!(KnobStyle::default(), KnobStyle::LAVENDER);
}

/// A style is nothing but geometry: the cell always spans the dial plus
/// its paddings, whatever diameter a plugin picks.
#[test]
fn cell_tracks_the_dial_diameter() {
    let style = KnobStyle {
        diameter: 38.0,
        pad_x: 12.0,
        text_h: 30.0,
        ..KnobStyle::LAVENDER
    };
    let cell = style.cell();
    assert_eq!((cell.x, cell.y), (50.0, 68.0));
}
