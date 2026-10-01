//! The editor must fit its own window (drums-plugin-rework.md §1.3, §6.1).
//!
//! These are headless CPU-only egui frames, the same pattern as
//! `resonance-plugin/tests/library_ui_render.rs`: run the editor's
//! `EditorApp::ui` through `egui::Context::run_ui` and read back what it
//! painted. `DrumsEditorApp` is private outside the crate, so this goes
//! through the `test_render_editor_frame` hook (`editor/mod.rs`).
//!
//! # Why "drawn" is not enough
//!
//! The first version of this file only checked that each label's text
//! showed up in the frame's shape list. That cannot fail: `Painter::text`
//! emits its shape whatever the clip rect, so a label laid out off the
//! window's edge, or under a clip rect of zero (or negative) width, is
//! still "drawn". The Pads body shipped laid out sideways — every card's
//! contents running left to right, the GLOBAL card at x = 1016–1417 in a
//! 960 px window under an inverted clip, the pad rows under a 0 px clip —
//! and that test passed throughout.
//!
//! So every check here is on what the user can actually *see*: a rect
//! intersected with the clip it was painted under, and with the window.
#![cfg(feature = "editor")]

use plugin_gui_core::egui;
use resonance_drums::download::WorkerConfig;
use resonance_drums::library::{self, Roots, SharedKitLibrary};
use resonance_drums::{EditorFrameProbe, ResonanceDrums, TestEditor};
use resonance_plugin::ResonancePlugin;

/// §6.1: 960×640 default, 780×520 minimum — the exact pair `factory.rs`
/// declares.
const SIZES: [(f32, f32); 2] = [(960.0, 640.0), (780.0, 520.0)];

/// The label of each always-visible control: MASTER on the KIT card,
/// POLYPHONY, VELOCITY CURVE and ROUND ROBIN on the GLOBAL card.
const GLOBAL_LABELS: [&str; 4] = ["MASTER", "POLYPHONY", "VELOCITY CURVE", "ROUND ROBIN"];

/// The control widget each of those labels heads, as `app.rs` probes it.
const GLOBAL_WIDGETS: [&str; 4] = [
    "kit.master",
    "global.polyphony",
    "global.velocity_curve",
    "global.round_robin",
];

/// Antialiasing and glyph bounds can poke a fraction of a pixel past a
/// clip that still shows the whole thing.
const TOLERANCE: f32 = 1.0;

/// `rect` is fully visible: non-empty, inside the clip it was drawn under,
/// and inside the window.
fn fully_visible(rect: egui::Rect, clip: egui::Rect, screen: egui::Rect) -> Result<(), String> {
    let visible = rect.intersect(clip).intersect(screen);
    if !(visible.width() > 0.0 && visible.height() > 0.0) {
        return Err(format!(
            "nothing of it is visible: rect {rect:?}, clip {clip:?}, screen {screen:?}"
        ));
    }
    if !clip.expand(TOLERANCE).contains_rect(rect) {
        return Err(format!("clipped: rect {rect:?} is not inside its clip {clip:?}"));
    }
    if !screen.expand(TOLERANCE).contains_rect(rect) {
        return Err(format!("off-window: rect {rect:?} is not inside {screen:?}"));
    }
    Ok(())
}

fn frames() -> Vec<((f32, f32), EditorFrameProbe)> {
    let plugin = ResonanceDrums::new();
    SIZES
        .iter()
        .map(|&size| (size, resonance_drums::test_render_editor_frame(&plugin, size)))
        .collect()
}

/// The text `needle` was painted at least once fully visible.
fn assert_text_visible(frame: &EditorFrameProbe, size: (f32, f32), needle: &str) {
    let hits: Vec<_> = frame.texts.iter().filter(|t| t.text == needle).collect();
    assert!(!hits.is_empty(), "{needle:?} is not drawn at all at {size:?}");
    let errors: Vec<String> = hits
        .iter()
        .filter_map(|t| fully_visible(t.rect, t.clip, frame.screen).err())
        .collect();
    assert!(
        errors.len() < hits.len(),
        "{needle:?} is drawn but not visible at {size:?}: {errors:?}"
    );
}

#[test]
fn every_global_label_is_visible_at_both_window_sizes() {
    for (size, frame) in frames() {
        for label in GLOBAL_LABELS {
            assert_text_visible(&frame, size, label);
        }
        // The round-robin control's own segments, not just its heading:
        // "Random" was the one clipped off the GLOBAL card's right edge.
        for segment in resonance_drums::params::ROUND_ROBIN_LABELS {
            assert_text_visible(&frame, size, segment);
        }
    }
}

#[test]
fn every_global_control_widget_is_visible_at_both_window_sizes() {
    for (size, frame) in frames() {
        for name in GLOBAL_WIDGETS {
            let w = frame
                .widget(name)
                .unwrap_or_else(|| panic!("{name} was not laid out at {size:?}"));
            if let Err(e) = fully_visible(w.rect, w.clip, frame.screen) {
                panic!("{name} is not visible at {size:?}: {e}");
            }
        }
    }
}

/// Each control's heading is a label on the left and its current value on
/// the right. Both must be visible and must not overlap — at the minimum
/// width VELOCITY CURVE's value used to be drawn over its own label.
#[test]
fn global_headings_show_label_and_value_side_by_side() {
    for (size, frame) in frames() {
        for label in GLOBAL_LABELS {
            let l = frame
                .widget(&format!("{label}.label"))
                .unwrap_or_else(|| panic!("{label}'s label was not probed at {size:?}"));
            let v = frame
                .widget(&format!("{label}.value"))
                .unwrap_or_else(|| panic!("{label}'s value was not probed at {size:?}"));
            for (what, r) in [("label", l), ("value", v)] {
                if let Err(e) = fully_visible(r.rect, r.clip, frame.screen) {
                    panic!("{label}'s {what} is not visible at {size:?}: {e}");
                }
            }
            assert!(
                l.rect.right() <= v.rect.left() + TOLERANCE,
                "{label}'s value overlaps its label at {size:?}: label {:?}, value {:?}",
                l.rect,
                v.rect
            );
        }
    }
}

/// A heading's value is elided with "…" when its column is too narrow
/// for it. On the GLOBAL card that should never be needed at a size the
/// editor declares: each value is as wide at the minimum as at the
/// default, i.e. shown in full at both.
#[test]
fn global_values_are_not_elided_at_the_minimum_size() {
    let all = frames();
    let (_, default) = &all[0];
    let (min_size, minimum) = &all[1];
    for label in ["POLYPHONY", "VELOCITY CURVE", "ROUND ROBIN"] {
        let name = format!("{label}.value");
        let full = default.widget(&name).unwrap().rect.width();
        let small = minimum.widget(&name).unwrap().rect.width();
        assert!(
            (full - small).abs() < 0.5,
            "{label}'s value is elided at {min_size:?}: {small} px wide against {full} px"
        );
    }
}

/// A kit-load error is the longest thing the KIT heading ever shows
/// ("Error: " plus up to 80 characters, ~550 px). It is elided to fit
/// beside its label, never painted over it.
#[test]
fn a_long_kit_error_does_not_overrun_its_label() {
    let plugin = ResonanceDrums::new();
    *plugin.bridge.kit_status.lock() = resonance_drums::kit_loader::KitStatus::Error {
        message: "no drum_samples.json found in /a/very/long/path/to/a/kit/that/goes/on/and/on"
            .to_string(),
    };
    for size in SIZES {
        let frame = resonance_drums::test_render_editor_frame(&plugin, size);
        let l = frame.widget("KIT.label").expect("KIT label");
        let v = frame.widget("KIT.value").expect("KIT value");
        for (what, r) in [("label", l), ("value", v)] {
            if let Err(e) = fully_visible(r.rect, r.clip, frame.screen) {
                panic!("the KIT {what} is not visible at {size:?}: {e}");
            }
        }
        assert!(
            l.rect.right() <= v.rect.left() + TOLERANCE,
            "the error overlaps the KIT label at {size:?}: label {:?}, value {:?}",
            l.rect,
            v.rect
        );
        let card = frame.widget("card.kit").expect("KIT card");
        assert!(
            v.rect.right() <= card.rect.right() + TOLERANCE,
            "the error runs out of the KIT card at {size:?}: {:?} vs {:?}",
            v.rect,
            card.rect
        );
    }
}

/// The pad list's rows have real width on screen — they were laid out
/// under a 0 px clip while the body ran sideways.
#[test]
fn pad_list_rows_have_visible_width() {
    for (size, frame) in frames() {
        let rows: Vec<_> = frame
            .widgets
            .iter()
            .filter(|w| w.name.starts_with("pad_row."))
            .collect();
        assert!(!rows.is_empty(), "no pad rows were laid out at {size:?}");
        let first = rows[0];
        let visible = first.rect.intersect(first.clip).intersect(frame.screen);
        assert!(
            visible.width() > 100.0 && visible.height() > 0.0,
            "the first pad row is not visible at {size:?}: rect {:?}, clip {:?}",
            first.rect,
            first.clip
        );
    }
}

/// The inspector fits beside the pad list — it used to be laid out
/// 1878 px wide in a 960 px window.
#[test]
fn the_inspector_fits_the_window() {
    for (size, frame) in frames() {
        let inspector = frame
            .widget("inspector")
            .unwrap_or_else(|| panic!("the inspector was not laid out at {size:?}"));
        assert!(
            inspector.rect.width() <= frame.screen.width()
                && inspector.rect.right() <= frame.screen.right() + TOLERANCE
                && inspector.rect.left() >= frame.screen.left(),
            "the inspector frame does not fit the {size:?} window: {:?}",
            inspector.rect
        );
    }
}

/// At the minimum size the editor must still draw a real frame and not
/// panic — a window narrower/shorter than a card's own margins used to
/// be able to make `Ui::set_min_width`'s debug assert fire (ba todo
/// #1377).
#[test]
fn the_editor_renders_something_at_the_minimum_size() {
    let plugin = ResonanceDrums::new();
    let drawn = resonance_drums::test_render_editor_frame(&plugin, (780.0, 520.0));
    assert!(!drawn.texts.is_empty(), "the editor drew nothing at its minimum size");
}

/// Below the minimum, the window should still degrade — scroll, clip,
/// whatever — rather than panic. The runtime clamps to `MIN_SIZE`, but a
/// host is free to ask for less while a resize is in flight.
#[test]
fn the_editor_does_not_panic_below_its_minimum_size() {
    let plugin = ResonanceDrums::new();
    let _ = resonance_drums::test_render_editor_frame(&plugin, (400.0, 300.0));
}

/// The header's controls fit at both declared sizes (§6.1): `Library…`
/// (which replaced "Download kits…" and "Open kit file…"), the kit
/// dropdown and, with a library kit loaded, its ☆/★.
#[test]
fn the_header_controls_fit_at_both_window_sizes() {
    for (size, frame) in frames() {
        for name in ["header.library", "kit.combo"] {
            let w = frame
                .widget(name)
                .unwrap_or_else(|| panic!("{name} was not laid out at {size:?}"));
            if let Err(e) = fully_visible(w.rect, w.clip, frame.screen) {
                panic!("{name} is not visible at {size:?}: {e}");
            }
        }
        assert_text_visible(&frame, size, "Library…");
    }

    // With a library kit loaded, the star joins the kit bar.
    let base = std::env::temp_dir().join(format!(
        "resonance-drums-layout-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    let kit_dir = base.join("drumkits/Layout Kit/layoutkit");
    std::fs::create_dir_all(&kit_dir).unwrap();
    let manifest = kit_dir.join("drum_samples.json");
    std::fs::write(
        &manifest,
        r#"{"Kick": {"01_KickIn": {"brand": "AKG", "channel": "01", "mic": "D112",
            "position": "KickIn", "rounds": {"RR01": {"Vel01": "k.wav"}}}},
            "_meta": {"name": "A Kit With A Rather Long Library Name"}}"#,
    )
    .unwrap();
    let library = SharedKitLibrary::open(Roots {
        root: Some(base.join("drumkits")),
        marks_dir: Some(base.join("library")),
        installed_json: None,
        worker: WorkerConfig {
            index_url: "http://127.0.0.1:9/index.json".into(),
            ..WorkerConfig::default()
        },
    });
    library.rescan().unwrap().unwrap();
    let plugin = ResonanceDrums::new();
    *plugin.bridge.kit_path.lock() = Some(manifest);
    for size in SIZES {
        let mut editor = TestEditor::new(&plugin, library.clone(), size);
        editor.frame(Vec::new());
        let frame = editor.frame(Vec::new());
        for name in ["header.library", "kit.combo", "kit.star"] {
            let w = frame
                .widget(name)
                .unwrap_or_else(|| panic!("{name} was not laid out at {size:?}"));
            if let Err(e) = fully_visible(w.rect, w.clip, frame.screen) {
                panic!("{name} is not visible at {size:?} with a kit loaded: {e}");
            }
        }
        let star = frame.widget("kit.star").unwrap().rect;
        let combo = frame.widget("kit.combo").unwrap().rect;
        assert!(
            star.right() <= combo.left() + TOLERANCE,
            "the star overlaps the kit dropdown at {size:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// The Library overlay fits the minimum window: its panel inside the
/// window and Close reachable, on either tab.
#[test]
fn the_library_overlay_fits_both_window_sizes() {
    let plugin = ResonanceDrums::new();
    for size in SIZES {
        library::isolate_for_tests();
        let mut editor = TestEditor::new(&plugin, library::shared(), size);
        editor.open_library();
        for plok in [true, false] {
            editor.show_tab(plok);
            editor.frame(Vec::new());
            let frame = editor.frame(Vec::new());
            let panel = frame.widget("library.panel").expect("the panel was laid out");
            assert!(
                frame.screen.expand(TOLERANCE).contains_rect(panel.rect),
                "the Library panel overflows the {size:?} window: {:?}",
                panel.rect
            );
            assert_text_visible(&frame, size, "Close");
            assert_text_visible(&frame, size, "KIT LIBRARY");
        }
    }
}
