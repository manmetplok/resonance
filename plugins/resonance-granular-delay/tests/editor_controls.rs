//! The granular strip's controls edit along each param's declared skew
//! and tell the host (code review PUX-01/-05), driven through headless
//! frames of the real editor app.
#![cfg(feature = "editor")]

use plugin_gui_core::egui;
use resonance_granular_delay::editor::headless_editor;
use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::ResonancePlugin;

const SIZE: (f32, f32) = (1320.0, 700.0);

#[test]
fn a_filter_knob_drag_follows_the_declared_skew_and_is_one_edit() {
    let plugin = ResonanceGranularDelay::new();
    let mut editor = headless_editor(&plugin, SIZE);
    let frame = editor.settled();
    let r = frame.widget("filter_hz").expect("Filter knob drawn").rect;
    let p = &plugin.params.filter_hz;
    let before = p.normalized_value();
    let from = egui::pos2(r.center().x, r.top() + 15.0);
    editor.drag(from, from + egui::vec2(0.0, 40.0), 20);
    let after = p.normalized_value();
    let moved = before - after;
    assert!((0.15..=0.21).contains(&moved), "travel {before} -> {after}");
    let want = p.plain_at_normalized(after);
    assert!(
        (p.value() - want).abs() < 1e-2,
        "Filter is {} Hz, the param's own curve says {want} Hz at travel {after}",
        p.value()
    );
    assert_eq!(editor.announced(), ["filter_hz"]);
}

#[test]
fn the_sync_chip_and_a_segment_announce() {
    let plugin = ResonanceGranularDelay::new();
    let mut editor = headless_editor(&plugin, SIZE);
    let frame = editor.settled();
    let sync = plugin.params.sync.value();
    editor.click(frame.widget("sync").expect("SYNC chip").rect.center());
    assert_eq!(plugin.params.sync.value(), !sync);
    let q = frame.widget("quality").expect("Quality segments").rect;
    // Three segments across; the left one is LO-FI (0).
    editor.click(egui::pos2(q.left() + 6.0, q.center().y));
    assert_eq!(plugin.params.quality.value(), 0);
    assert_eq!(editor.announced(), ["sync", "quality"]);
}
