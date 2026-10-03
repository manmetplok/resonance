//! The gate editor tells the host about its edits (code review PUX-01).
//!
//! Before, only the drums announced: a gate knob drag set the param and
//! the host never heard of it — no undo entry, a stale mirror, a stale
//! MCP readback. The gate's knobs are `editor_widgets::float_knob`, the
//! binding nine editors share, so this is the test of that path.
#![cfg(feature = "editor")]

use plugin_gui_core::egui;
use resonance_gate::editor::headless_editor;
use resonance_gate::ResonanceGate;
use resonance_plugin::ResonancePlugin;

#[test]
fn a_knob_drag_is_announced_once_when_it_ends() {
    let plugin = ResonanceGate::new();
    let mut editor = headless_editor(&plugin, (900.0, 420.0));
    let frame = editor.settled();
    let r = frame.widget("threshold").expect("threshold knob drawn").rect;
    let before = plugin.params.threshold.value();
    let from = egui::pos2(r.center().x, r.top() + 20.0);
    editor.drag(from, from - egui::vec2(0.0, 30.0), 15);
    assert!(plugin.params.threshold.value() > before, "the drag did not move the knob");
    assert_eq!(editor.announced(), ["threshold"], "one drag, one announced edit");
}
