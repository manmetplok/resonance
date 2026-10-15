//! The EQ's band sliders and curve nodes edit through the params' own
//! declared curves and announce their edits (code review PUX-01/-06),
//! driven through headless frames of the real editor app.
//!
//! The band strip restated Freq and Q as true-log ranges while the
//! params declare `FloatRange::Skewed`, passed no reset default (a
//! double-click did nothing) and had no typed entry; nothing told the
//! host about an edit.
//!
//! `layout` (BUG-41): the editor fits its window at the factory's sizes.
#![cfg(feature = "editor")]

#[path = "editor_controls/layout.rs"]
mod layout;

use plugin_gui_core::egui;
use resonance_eq::editor::headless_editor;
use resonance_eq::ResonanceEq;
use resonance_plugin::ResonancePlugin;

const SIZE: (f32, f32) = (1200.0, 760.0);

#[test]
fn a_freq_slider_drag_follows_the_declared_skew_and_is_one_edit() {
    let plugin = ResonanceEq::new();
    let mut editor = headless_editor(&plugin, SIZE);
    let frame = editor.settled();
    let band = &plugin.params.bands[2];
    let r = frame
        .widgets
        .iter()
        .find(|w| w.name == "band2_freq")
        .expect("band 3's Freq slider drawn")
        .rect;
    let before = band.freq.normalized_value();
    let from = r.center();
    editor.drag(from, from + egui::vec2(30.0, 0.0), 15);
    let after = band.freq.normalized_value();
    assert!(after > before, "Freq did not move");
    assert!(
        (band.freq.value() - band.freq.plain_at_normalized(after)).abs() < 0.5,
        "the slider's travel is not the param's own curve"
    );
    assert_eq!(editor.announced(), ["band2_freq"]);
}

#[test]
fn a_double_click_resets_a_band_slider() {
    let plugin = ResonanceEq::new();
    let band = &plugin.params.bands[2];
    band.q.set_value(5.0);
    let mut editor = headless_editor(&plugin, SIZE);
    let frame = editor.settled();
    let r = frame
        .widgets
        .iter()
        .find(|w| w.name == "band2_q")
        .expect("band 3's Q slider drawn")
        .rect;
    editor.double_click(r.center());
    assert_eq!(band.q.value(), band.q.default_value());
    assert_eq!(editor.announced(), ["band2_q"]);
}

#[test]
fn a_typed_freq_lands_exactly() {
    let plugin = ResonanceEq::new();
    let mut editor = headless_editor(&plugin, SIZE);
    let frame = editor.settled();
    let r = frame
        .widgets
        .iter()
        .find(|w| w.name == "band2_freq.value")
        .expect("band 3's Freq readout drawn")
        .rect;
    editor.click(r.center());
    editor.type_and_enter("440");
    assert_eq!(plugin.params.bands[2].freq.value(), 440.0);
    assert_eq!(editor.announced(), ["band2_freq"]);
}

/// A cut band's stage Qs come from its slope (a Butterworth cascade), so
/// the strip draws no Q slider there — one would move nothing (BUG-40).
/// A bell still gets one.
#[test]
fn cut_bands_draw_no_q_slider() {
    let plugin = ResonanceEq::new();
    plugin.params.bands[1].kind.set_value(0); // Bell
    plugin.params.bands[2].kind.set_value(3); // LowCut
    plugin.params.bands[3].kind.set_value(4); // HighCut
    let mut editor = headless_editor(&plugin, SIZE);
    let frame = editor.settled();
    let drawn = |name: &str| frame.widgets.iter().any(|w| w.name == name);
    assert!(drawn("band1_q"), "a bell shows its Q");
    assert!(!drawn("band2_q"), "a low cut shows a Q that does nothing");
    assert!(!drawn("band3_q"), "a high cut shows a Q that does nothing");
}
