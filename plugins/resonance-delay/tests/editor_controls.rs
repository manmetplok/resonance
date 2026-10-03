//! The delay editor's controls edit their params and tell the host
//! (code review PUX-01/-02/-05/-06), driven through headless frames of
//! the real editor app.
#![cfg(feature = "editor")]

use plugin_gui_core::egui;
use resonance_delay::editor::headless_editor;
use resonance_delay::ResonanceDelay;
use resonance_plugin::editor_widgets::headless::{FrameProbe, HeadlessEditor};
use resonance_plugin::ResonancePlugin;

const SIZE: (f32, f32) = (1200.0, 600.0);

fn open(plugin: &ResonanceDelay) -> (HeadlessEditor, FrameProbe) {
    let mut editor = headless_editor(plugin, SIZE);
    let frame = editor.settled();
    (editor, frame)
}

fn rect_of(frame: &FrameProbe, id: &str) -> egui::Rect {
    frame
        .widget(id)
        .unwrap_or_else(|| panic!("`{id}` is not drawn"))
        .rect
}

/// The dial's centre (the top of a knob cell).
fn dial(frame: &FrameProbe, id: &str) -> egui::Pos2 {
    let r = rect_of(frame, id);
    egui::pos2(r.center().x, r.top() + 20.0)
}

#[test]
fn sync_freeze_and_gate_switch_with_a_click_and_announce() {
    let plugin = ResonanceDelay::new();
    let (mut editor, frame) = open(&plugin);
    assert!(plugin.params.sync.value(), "sync defaults on");
    for id in ["sync", "freeze", "gate_on"] {
        let at = rect_of(&frame, id).center();
        editor.click(at);
    }
    assert!(!plugin.params.sync.value(), "Sync did not switch off");
    assert!(plugin.params.freeze.value(), "Freeze did not engage");
    assert!(plugin.params.gate_on.value(), "Gate did not switch on");
    assert_eq!(editor.announced(), ["sync", "freeze", "gate_on"]);
}

#[test]
fn routing_picks_a_segment_and_announces() {
    let plugin = ResonanceDelay::new();
    let (mut editor, frame) = open(&plugin);
    let r = rect_of(&frame, "routing");
    // Vertical: Stereo, Ping-Pong, Dual. The bottom segment is Dual.
    editor.click(egui::pos2(r.center().x, r.bottom() - 4.0));
    assert_eq!(plugin.params.routing.value(), 2);
    assert_eq!(editor.announced(), ["routing"]);
}

#[test]
fn a_knob_drag_is_one_announced_edit_along_the_declared_skew() {
    let plugin = ResonanceDelay::new();
    let (mut editor, frame) = open(&plugin);
    let before = plugin.params.time_ms.normalized_value();
    let from = dial(&frame, "time_ms");
    // 40 px up in 2 px steps: +0.2 of travel.
    editor.drag(from, from - egui::vec2(0.0, 40.0), 20);
    let after = plugin.params.time_ms.normalized_value();
    // egui keeps the first few px of a press as click slop before it
    // calls it a drag, so a 40 px drag moves a little under 0.2.
    let moved = after - before;
    assert!(
        (0.15..=0.21).contains(&moved),
        "Time moved {before} -> {after} in travel, want ~+0.2 along the param's own curve"
    );
    // The plain value is the param's own answer for that travel: the
    // knob did not map linearly over 1..2000 ms.
    let plain = plugin.params.time_ms.value();
    let want = plugin.params.time_ms.plain_at_normalized(after);
    assert!((plain - want).abs() < 1e-3, "{plain} vs {want}");
    assert_eq!(editor.announced(), ["time_ms"], "one drag, one edit");
}

#[test]
fn a_double_click_resets_to_the_declared_default() {
    let plugin = ResonanceDelay::new();
    plugin.params.hi_cut.set_value(2_000.0);
    let (mut editor, frame) = open(&plugin);
    editor.double_click(dial(&frame, "hi_cut"));
    assert_eq!(plugin.params.hi_cut.value(), plugin.params.hi_cut.default_value());
    assert_eq!(editor.announced(), ["hi_cut"]);
}

#[test]
fn a_typed_value_lands_exactly_and_announces() {
    let plugin = ResonanceDelay::new();
    let (mut editor, frame) = open(&plugin);
    let r = rect_of(&frame, "duck_threshold");
    // The readout row under the dial (CAPTIONED: dial 40, value at +3).
    editor.click(egui::pos2(r.center().x, r.top() + 40.0 + 7.0));
    editor.type_and_enter("-12.5");
    assert_eq!(plugin.params.duck_threshold.value(), -12.5);
    assert_eq!(editor.announced(), ["duck_threshold"]);
}

#[test]
fn a_division_pick_announces() {
    let plugin = ResonanceDelay::new();
    let (mut editor, frame) = open(&plugin);
    let r = rect_of(&frame, "division");
    editor.click(r.center());
    let opened = editor.frame(Vec::new());
    let item = opened
        .texts
        .iter()
        .filter(|t| t.text == "1/8D")
        .last()
        .expect("the open combo lists 1/8D")
        .rect
        .center();
    editor.click(item);
    let want = resonance_delay::sync::DIVISION_LABELS
        .iter()
        .position(|l| *l == "1/8D")
        .unwrap() as i32;
    assert_eq!(plugin.params.division.value(), want);
    assert_eq!(editor.announced(), ["division"]);
}
