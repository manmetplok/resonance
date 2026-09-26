//! The editor must survive a layout narrower than its own margins
//! (ba todo #1377).
//!
//! Every card in the pad inspector sized itself as "whatever width is
//! available, less my frame's margins", with no floor. `available_width`
//! clamps at zero, so once it is under the margin the subtraction goes
//! negative, and `Ui::set_min_width` carries
//! `debug_assert!(0.0 <= width)`. That is a panic on the editor thread —
//! for a CLAP plugin, a dead window inside the host's process. It was
//! seen for real at `draw_articulations` while opening all eleven editors
//! against a live Wayland session.
//!
//! # What these tests do and do not prove
//!
//! Read this before trusting them, because two plausible versions of
//! this file prove nothing at all.
//!
//! * The **source guard** is what pins the fix. It fails against the
//!   unfixed code; the render tests below do not.
//! * The **render tests** exercise the inspector across pads and column
//!   widths and assert it neither panics nor renders nothing. They could
//!   not be made to reproduce the original panic, and the reason is worth
//!   recording: any widget wider than its allocation calls
//!   `expand_to_include_x`, which grows the region's `max_rect`, so the
//!   inspector's own pad head and knob grid push `available_width` back
//!   out to ~264px before the articulations card is ever reached. Neither
//!   a 0px window nor a 0px `allocate_ui` column gets underneath that.
//!   Whatever produced a genuinely starved Ui in the live editor is not
//!   reproduced here.
//!
//! Severity, stated accurately: `debug_assert!` compiles out under
//! `--release`, and `scripts/bundle.sh` builds `--release` with the
//! workspace's default `debug-assertions = false`. egui's placer discards
//! a non-positive width by itself. So a **shipped bundle never panicked**
//! — this killed debug builds, which is to say developers and anyone
//! running the editors from a dev tree.
#![cfg(feature = "editor")]

use plugin_gui_core::egui;
use resonance_drums::ResonanceDrums;
use resonance_plugin::ResonancePlugin;

/// Render the pad inspector for `pad` into a column `width` points
/// across.
///
/// The column matters more than the window. `app.rs` draws the inspector
/// inside `ui.allocate_ui(vec2(right_w, ..))`, and an allocated child Ui
/// is CLIPPED to its allocation — unlike a free-standing one, whose
/// content can push `available_width` back out. Drawn free-standing at a
/// 0px window the inspector reports 264px available and never goes
/// negative, so a test that skips the allocation cannot reproduce this
/// at any window size. That is the trap this helper exists to avoid.
///
/// Returns the number of shapes the frame produced, so a caller can tell
/// "laid out in a tiny space" apart from "drew nothing at all".
fn draw_at(plugin: &ResonanceDrums, pad: usize, width: f32) -> usize {
    let ctx = egui::Context::default();
    // A window far wider than the column, so the column is unambiguously
    // what constrains the inspector.
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0));
    let input = egui::RawInput {
        screen_rect: Some(screen),
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show_inside(ui, |ui| {
                ui.allocate_ui(egui::vec2(width, 700.0), |ui| {
                    resonance_drums::test_draw_pad_inspector(ui, &plugin.bridge, pad);
                });
            });
    });
    ctx.tessellate(output.shapes, output.pixels_per_point).len()
}

/// Every pad at every hostile column width, drawn without panicking.
///
/// 28.0 is the inspector frame's own horizontal margin, so those are the
/// widths where the unfloored subtraction would go negative if the
/// region did not expand first (see the module doc — it does).
///
/// Every pad, not just the first: the articulations card — the one the
/// crash was observed in — is drawn only for pads whose mapping declares
/// an alternate piece, and pad 0's does not. Five of the sixteen do, so a
/// single-pad loop would skip the interesting card entirely.
#[test]
fn the_pad_inspector_survives_a_column_narrower_than_its_margins() {
    let plugin = ResonanceDrums::new();
    for pad in 0..resonance_drums::drum_map::NUM_PADS {
        for width in [0.0, 1.0, 12.0, 27.0, 28.0, 29.0, 40.0] {
            draw_at(&plugin, pad, width);
        }
    }
}

/// ...and still draws at a width a person would actually use, so the
/// floor did not turn the fix into "render nothing".
#[test]
fn the_pad_inspector_still_lays_out_at_a_normal_width() {
    let plugin = ResonanceDrums::new();
    assert!(
        draw_at(&plugin, 0, 900.0) > 0,
        "the inspector produced no shapes at 900px — the floor cannot be \
         suppressing the whole card"
    );
    // A squeezed window still paints its frame and whatever text fits;
    // the point is that it degrades rather than disappearing or dying.
    assert!(
        draw_at(&plugin, 0, 20.0) > 0,
        "a narrow window must still render"
    );
}

/// The guard against the idiom coming back. `available_width()` minus a
/// constant is the shape that caused this, and it was in eight places
/// across two files — copied from card to card, which is exactly how it
/// will return.
#[test]
fn no_editor_source_subtracts_from_the_available_width_unfloored() {
    const SOURCES: &[(&str, &str)] = &[
        ("app.rs", include_str!("../src/editor/app.rs")),
        ("chrome.rs", include_str!("../src/editor/chrome.rs")),
        ("pad_grid.rs", include_str!("../src/editor/pad_grid.rs")),
        (
            "pad_inspector.rs",
            include_str!("../src/editor/pad_inspector.rs"),
        ),
        ("kit_browser.rs", include_str!("../src/editor/kit_browser.rs")),
    ];

    for (name, src) in SOURCES {
        for (n, line) in src.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            assert!(
                !code.contains("available_width() -"),
                "{name}:{} subtracts from available_width() directly. Use \
                 `body_width(ui, inset)`, which floors at zero — a window \
                 narrower than a card's margins is not a bug the user can \
                 avoid (ba todo #1377).\n  {}",
                n + 1,
                line.trim()
            );
        }
    }
}
