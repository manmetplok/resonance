//! The wavetable editor's discrete knobs step on an ordinary drag and
//! every edit is announced to the host (code review PUX-01/-02), driven
//! through headless frames of the real editor app.
#![cfg(feature = "editor")]

use plugin_gui_core::egui;
use resonance_plugin::{Param, ResonancePlugin};
use resonance_wavetable::editor::headless_editor;
use resonance_wavetable::ResonanceWavetable;

fn param<'a>(plugin: &'a ResonanceWavetable, id: &str) -> &'a dyn Param {
    (0..plugin.param_count())
        .map(|i| plugin.param(i))
        .find(|p| p.id() == id)
        .unwrap_or_else(|| panic!("no param {id}"))
}

/// PUX-02: the distortion Mode knob needed ~20 px of drag in a single
/// frame to step; 120 px in 2 px steps did nothing.
#[test]
fn a_slow_drag_steps_the_distortion_mode() {
    let plugin = ResonanceWavetable::new();
    let mut editor = headless_editor(&plugin, (960.0, 560.0), true);
    let frame = editor.settled();
    let r = frame.widget("dist_mode").expect("the FX tab draws Mode").rect;
    let before = param(&plugin, "dist_mode").get_plain();
    let from = egui::pos2(r.center().x, r.top() + 20.0);
    editor.drag(from, from - egui::vec2(0.0, 120.0), 60);
    let after = param(&plugin, "dist_mode").get_plain();
    assert!(after > before, "Mode stayed at {before} after a 120 px drag");
    assert_eq!(editor.announced(), ["dist_mode"]);
}

/// A slider double-click resets to the declared default (the binding
/// supplies it, PUX-06) and announces the edit.
#[test]
fn a_float_slider_reset_announces() {
    let plugin = ResonanceWavetable::new();
    let mut editor = headless_editor(&plugin, (960.0, 560.0), false);
    let frame = editor.settled();
    let balance = param(&plugin, "osc_balance");
    balance.set_plain(balance.max_plain());
    let r = frame.widget("osc_balance").expect("the Osc tab draws Balance").rect;
    editor.double_click(r.center());
    assert_eq!(balance.get_plain(), balance.default_plain());
    assert_eq!(editor.announced(), ["osc_balance"]);
}
