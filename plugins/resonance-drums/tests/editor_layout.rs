//! The editor must fit its own window (drums-plugin-rework.md §1.3, §6.1).
//!
//! The body used to need about 640px of height and get 302 — the GLOBAL
//! card (POLYPHONY, VELOCITY CURVE, ROUND ROBIN) and the KIT card (MASTER)
//! were permanently below the window edge, and nothing scrolled to reach
//! them. `app.rs` now gives the KIT/GLOBAL row a fixed bottom panel and
//! wraps the pad list and the inspector in their own scroll areas, so
//! every global control has its own guaranteed slice of the window
//! regardless of how tall the pad content gets.
//!
//! These are headless CPU-only egui frames, the same pattern as
//! `resonance-plugin/tests/library_ui_render.rs`: run one frame through
//! `egui::Context::run_ui` and read back the text shapes it painted.
//! `DrumsEditorApp` itself is private outside the crate, so this goes
//! through the `test_render_editor_frame` hook (`editor/mod.rs`) rather
//! than constructing the app directly.
#![cfg(feature = "editor")]

use resonance_drums::ResonanceDrums;
use resonance_plugin::ResonancePlugin;

/// §6.1: 960×640 default, 780×520 minimum — the exact pair `factory.rs`
/// now declares.
const SIZES: [(f32, f32); 2] = [(960.0, 640.0), (780.0, 520.0)];

/// MASTER lives on the KIT card; POLYPHONY, VELOCITY CURVE and ROUND
/// ROBIN on the GLOBAL card next to it. Both cards sit in the body's
/// fixed bottom row, so none of them should ever depend on scroll
/// position to be drawn.
const GLOBAL_LABELS: [&str; 4] = ["MASTER", "POLYPHONY", "VELOCITY CURVE", "ROUND ROBIN"];

#[test]
fn every_global_control_is_reachable_at_both_window_sizes() {
    let plugin = ResonanceDrums::new();
    for size in SIZES {
        let drawn = resonance_drums::test_render_editor_frame(&plugin, size);
        for label in GLOBAL_LABELS {
            assert!(
                drawn.iter().any(|t| t == label),
                "{label} is not reachable at {size:?}: {drawn:?}"
            );
        }
    }
}

/// At the minimum size the editor must still draw a real frame and not
/// panic — a window narrower/shorter than a card's own margins used to
/// be able to make `Ui::set_min_width`'s debug assert fire (ba todo
/// #1377), and a layout bug here is exactly the kind that only shows up
/// at the smallest size a host actually lets the window reach.
#[test]
fn the_editor_renders_something_at_the_minimum_size() {
    let plugin = ResonanceDrums::new();
    let drawn = resonance_drums::test_render_editor_frame(&plugin, (780.0, 520.0));
    assert!(!drawn.is_empty(), "the editor drew nothing at its minimum size");
}

/// Below the minimum, the window should still degrade — scroll, clip,
/// whatever — rather than panic. The runtime clamps to `MIN_SIZE`, but a
/// host is free to ask for less while a resize is in flight.
#[test]
fn the_editor_does_not_panic_below_its_minimum_size() {
    let plugin = ResonanceDrums::new();
    let _ = resonance_drums::test_render_editor_frame(&plugin, (400.0, 300.0));
}
