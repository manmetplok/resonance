//! The Download Kits overlay's modal mechanics (drums-plugin-rework.md
//! §1.2, §6.5, §9).
//!
//! The backdrop used to be painted on an `Order::Tooltip` layer, which in
//! egui draws *above* `Order::Foreground` — the panel's own layer — so
//! opening the overlay gave a near-black screen (§1.2). It also was not
//! modal: clicks fell through to the pads behind it, and Esc did nothing.
//! `download_panel.rs` is now an `egui::Modal`, which keeps the backdrop
//! and the panel on the same layer and makes the backdrop (and the panel
//! itself) sense clicks so they stop short of the pads.
//!
//! `editor_honesty.rs` covers the `Order::Tooltip` regression as a source
//! guard; this file proves the paint order and the Esc behaviour at
//! runtime, through the `test_run_download_panel_frame` hook
//! (`editor/mod.rs` — `download_panel` is a private module outside this
//! crate).
#![cfg(feature = "editor")]

use plugin_gui_core::egui;
use resonance_drums::ResonanceDrums;
use resonance_plugin::ResonancePlugin;

fn shape_contains_text(shape: &egui::Shape, needle: &str) -> bool {
    match shape {
        egui::Shape::Text(t) => t.galley.text() == needle,
        egui::Shape::Vec(v) => v.iter().any(|s| shape_contains_text(s, needle)),
        _ => false,
    }
}

/// A black, partially-opaque fill covering (at least) the whole screen —
/// the backdrop, whatever its exact alpha. `download_panel::draw` asks
/// for a fixed alpha of 180, matching the amp's own overlay
/// (`resonance-amp/src/editor/library_panel.rs`), but `egui::Modal`
/// fades its backdrop in, so a freshly-opened panel's first settled
/// frame can report less than that — the shape to look for is "black,
/// not fully transparent, full-screen", not an exact colour.
fn shape_is_backdrop(shape: &egui::Shape, screen: egui::Rect) -> bool {
    match shape {
        egui::Shape::Rect(r) => {
            r.fill.r() == 0
                && r.fill.g() == 0
                && r.fill.b() == 0
                && r.fill.a() > 0
                && r.rect.contains_rect(screen)
        }
        egui::Shape::Vec(v) => v.iter().any(|s| shape_is_backdrop(s, screen)),
        _ => false,
    }
}

fn escape_event() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// The exact regression from §1.2: the dimming rect must come *before*
/// the panel's own content in the frame's shape list — shapes paint in
/// list order, so anything found earlier sits visually underneath
/// anything found later. A backdrop found after the header text would be
/// the bug coming back.
#[test]
fn the_backdrop_is_painted_below_the_panel_not_above_it() {
    let plugin = ResonanceDrums::new();
    let size = (960.0, 640.0);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1));
    let (_, shapes) = resonance_drums::test_run_download_panel_frame(&plugin, size, vec![], true);

    let backdrop = shapes
        .iter()
        .position(|s| shape_is_backdrop(&s.shape, screen))
        .expect("the overlay must dim the background while open");
    let header = shapes
        .iter()
        .position(|s| shape_contains_text(&s.shape, "DOWNLOAD KITS"))
        .expect("the panel header must be drawn while open");

    assert!(
        backdrop < header,
        "the backdrop (shape {backdrop}) must be painted before — and so sit below — the \
         panel's own content (shape {header}); it was found after it instead"
    );
}

#[test]
fn escape_closes_the_overlay() {
    let plugin = ResonanceDrums::new();
    let (open_after, _) = resonance_drums::test_run_download_panel_frame(
        &plugin,
        (960.0, 640.0),
        vec![escape_event()],
        true,
    );
    assert!(!open_after, "Esc should close the Download Kits overlay");
}

/// The negative case: without Esc, nothing closes the panel from under
/// the user.
#[test]
fn without_escape_the_overlay_stays_open() {
    let plugin = ResonanceDrums::new();
    let (open_after, _) =
        resonance_drums::test_run_download_panel_frame(&plugin, (960.0, 640.0), vec![], true);
    assert!(open_after, "the overlay closed on its own with no input");
}

/// A closed panel draws nothing — in particular, no leftover backdrop
/// darkening the editor behind it.
#[test]
fn a_closed_panel_draws_no_backdrop() {
    let plugin = ResonanceDrums::new();
    let size = (960.0, 640.0);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1));
    let (open_after, shapes) =
        resonance_drums::test_run_download_panel_frame(&plugin, size, vec![], false);
    assert!(!open_after);
    assert!(
        !shapes.iter().any(|s| shape_is_backdrop(&s.shape, screen)),
        "a closed overlay must not dim the editor behind it"
    );
}
