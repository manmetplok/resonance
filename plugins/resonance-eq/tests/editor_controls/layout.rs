//! The EQ editor fits its own window (BUG-41), measured on headless
//! frames of the real editor app at the sizes its factory declares.
//!
//! The band strip was a fixed 350 px while a band column lays out about
//! 400 px tall, so the Att and Rel sliders were cut off; eight 130 px
//! columns ran past the 960 px default width; and the header's one row
//! pushed the Output readout and the auto-gain switch off the right edge
//! of a narrow window. Every control here is a probed rect — the
//! param-bound widgets report themselves, and the editor probes each band
//! column, the strip's visible view and each header group — so "fits"
//! means the rect lies inside its clip and the window.

use plugin_gui_core::egui;
use resonance_eq::editor::{headless_editor, INITIAL_SIZE, MIN_SIZE};
use resonance_eq::ResonanceEq;
use resonance_plugin::editor_widgets::headless::{FrameProbe, HeadlessEditor};
use resonance_plugin::ResonancePlugin;

/// Rounding slack for a rect edge, px.
const TOL: f32 = 0.5;

/// The response curve keeps at least this much height, px, or the
/// strip "fits" by squeezing the editor's main view away.
const MIN_CURVE_H: f32 = 150.0;

fn size(s: (u32, u32)) -> (f32, f32) {
    (s.0 as f32, s.1 as f32)
}

/// The plugin in the band setups that change the column: every band a
/// bell (gain, Q and dynamics drawn), the factory default (cuts at the
/// ends: slope combo, no Q), each with and without the auto-gain
/// readout in the header.
fn setups() -> Vec<(&'static str, ResonanceEq)> {
    let mut out = Vec::new();
    for auto_gain in [false, true] {
        let default = ResonanceEq::new();
        default.params.auto_gain.set_value(auto_gain);
        out.push((if auto_gain { "default+auto" } else { "default" }, default));
        let bells = ResonanceEq::new();
        for band in &bells.params.bands {
            band.kind.set_value(0); // Bell
            band.dyn_on.set_value(true);
        }
        bells.params.auto_gain.set_value(auto_gain);
        out.push((if auto_gain { "bells+auto" } else { "bells" }, bells));
    }
    out
}

/// Frames enough for the layout to settle: the header measures its
/// groups on the first, wraps them on the second, and its panel takes
/// the new height on the third.
fn settle(editor: &mut HeadlessEditor) -> FrameProbe {
    editor.settled();
    editor.settled()
}

fn probe(frame: &FrameProbe, name: &str) -> egui::Rect {
    frame
        .widget(name)
        .unwrap_or_else(|| panic!("{name} not drawn"))
        .rect
}

/// `(bottom of the header, top of the strip)`: the curve's share of the
/// window lies between them.
fn curve_span(frame: &FrameProbe) -> (f32, f32) {
    let header_bottom = frame
        .widgets
        .iter()
        .filter(|w| w.name.starts_with("eq_header_"))
        .map(|w| w.rect.max.y)
        .fold(0.0, f32::max);
    let strip_top = probe(frame, "eq_band_strip_view").min.y;
    (header_bottom, strip_top)
}

/// The strip fits vertically: every column, Release readout and frame
/// included, inside the visible view, and the view plus the scroll bar it
/// may need inside the strip panel.
fn assert_strip_height_covers_columns(frame: &FrameProbe, what: &str) {
    let view = frame.widget("eq_band_strip_view").expect("strip view probed");
    let strip = frame.widget("eq_band_strip").expect("strip probed");
    assert!(
        strip.clip.expand(TOL).contains_rect(strip.rect),
        "{what}: the strip {:?}, scroll bar included, runs past its panel {:?}",
        strip.rect,
        strip.clip
    );
    for band in 0..8 {
        let column = probe(frame, &format!("eq_band_column{band}"));
        assert!(
            column.min.y >= view.rect.min.y - TOL && column.max.y <= view.rect.max.y + TOL,
            "{what}: band {} column {column:?} is taller than the strip view {:?}",
            band + 1,
            view.rect
        );
    }
}

/// At the size the editor opens at, nothing is clipped or off-window —
/// all eight band columns side by side, no scrolling — and the curve keeps
/// its room.
#[test]
fn every_control_is_visible_at_the_initial_size() {
    for (what, plugin) in setups() {
        let mut editor = headless_editor(&plugin, size(INITIAL_SIZE));
        let frame = settle(&mut editor);
        let hidden = frame.hidden_widgets(TOL);
        assert!(hidden.is_empty(), "{what}: hidden at {INITIAL_SIZE:?}: {hidden:#?}");
        // No scroll bar either: the row is no wider than its view. A
        // trailing gap after the last column once made it 4 px too wide.
        let view = probe(&frame, "eq_band_strip_view");
        let content = probe(&frame, "eq_band_strip_content");
        assert!(
            content.width() <= view.width() + TOL,
            "{what}: the band row {content:?} overflows its view {view:?} at {INITIAL_SIZE:?}"
        );
        assert_strip_height_covers_columns(&frame, what);
        let (header_bottom, strip_top) = curve_span(&frame);
        assert!(
            strip_top - header_bottom >= MIN_CURVE_H,
            "{what}: the curve gets {} px",
            strip_top - header_bottom
        );
    }
}

/// At the minimum size the band row scrolls sideways, but nothing is cut
/// off vertically, the header wraps rather than clipping, and the curve
/// keeps its room.
#[test]
fn the_minimum_size_wraps_the_header_and_scrolls_the_band_row() {
    for (what, plugin) in setups() {
        let mut editor = headless_editor(&plugin, size(MIN_SIZE));
        let frame = settle(&mut editor);
        assert_strip_height_covers_columns(&frame, what);

        // The header is never scrolled: everything in it is fully shown.
        let header_hidden: Vec<_> = frame
            .hidden_widgets(TOL)
            .into_iter()
            .filter(|(name, _)| {
                name.starts_with("eq_header_")
                    || name.starts_with("output_gain")
                    || name.starts_with("auto_gain")
                    || name == "eq_auto_gain_readout"
            })
            .collect();
        assert!(
            header_hidden.is_empty(),
            "{what}: header clipped at {MIN_SIZE:?}: {header_hidden:#?}"
        );

        // The band controls: vertically inside their clip; sideways the
        // row scrolls, so the first band at least is fully on screen.
        for w in frame.widgets.iter().filter(|w| w.name.starts_with("band")) {
            assert!(
                w.rect.min.y >= w.clip.min.y - TOL && w.rect.max.y <= w.clip.max.y + TOL,
                "{what}: {} {:?} cut off vertically by {:?}",
                w.name,
                w.rect,
                w.clip
            );
        }
        let first_band: Vec<_> = frame
            .hidden_widgets(TOL)
            .into_iter()
            .filter(|(name, _)| name.starts_with("band0_"))
            .collect();
        assert!(first_band.is_empty(), "{what}: {first_band:#?}");

        let (header_bottom, strip_top) = curve_span(&frame);
        assert!(
            strip_top - header_bottom >= MIN_CURVE_H,
            "{what}: the curve gets {} px",
            strip_top - header_bottom
        );
    }
}

/// The row really is scrollable at the minimum width: a drag on the
/// scroll bar brings the last band into view.
#[test]
fn the_last_band_scrolls_into_view_at_the_minimum_width() {
    let plugin = ResonanceEq::new();
    let mut editor = headless_editor(&plugin, size(MIN_SIZE));
    let frame = settle(&mut editor);
    assert!(
        frame
            .hidden_widgets(TOL)
            .iter()
            .any(|(n, _)| n.starts_with("band7_")),
        "the last band already fits at {MIN_SIZE:?} — this test no longer exercises the scroll"
    );
    let view = probe(&frame, "eq_band_strip_view");
    let bar_y = view.max.y + egui::style::ScrollStyle::solid().allocated_width() / 2.0;
    let frame = editor.drag(
        egui::pos2(view.min.x + 20.0, bar_y),
        egui::pos2(view.max.x + 200.0, bar_y),
        10,
    );
    let hidden: Vec<_> = frame
        .hidden_widgets(TOL)
        .into_iter()
        .filter(|(name, _)| name.starts_with("band7_"))
        .collect();
    assert!(hidden.is_empty(), "band 8 still hidden after scrolling: {hidden:#?}");
}

/// A wide window keeps the header on one row.
#[test]
fn a_wide_window_keeps_the_header_on_one_row() {
    let plugin = ResonanceEq::new();
    plugin.params.auto_gain.set_value(true);
    let mut editor = headless_editor(&plugin, (1400.0, size(INITIAL_SIZE).1));
    let frame = settle(&mut editor);
    let rows: Vec<f32> = frame
        .widgets
        .iter()
        .filter(|w| w.name.starts_with("eq_header_"))
        .map(|w| w.rect.min.y)
        .collect();
    assert_eq!(rows.len(), 5, "five header groups");
    assert!(
        rows.iter().all(|y| (y - rows[0]).abs() < 1.0),
        "header wrapped at 1400 px: {rows:?}"
    );
}
