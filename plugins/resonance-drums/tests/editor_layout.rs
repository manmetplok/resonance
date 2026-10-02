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
//! The K5 tabs are checked control by control, scrolled into view where
//! needed, in `editor_tabs.rs`; this file keeps the original regressions.
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
/// declares — and 1571×856, what a tiling compositor gives every plugin
/// editor on this machine (CLAUDE.md).
const SIZES: [(f32, f32); 3] = [(960.0, 640.0), (780.0, 520.0), (1571.0, 856.0)];

/// The caption of each global control, on the Mix tab's GLOBAL card (the
/// bottom-of-window KIT/GLOBAL cards are gone, K5).
const GLOBAL_LABELS: [&str; 4] = [
    "Polyphony",
    "Velocity curve",
    "Velocity humanize",
    "Round robin",
];

/// The control widget each of those captions heads, as `mix_tab.rs`
/// probes it — and the output mode switch, which heads the OUTPUTS card.
const GLOBAL_WIDGETS: [&str; 5] = [
    "global.polyphony",
    "global.velocity_curve",
    "global.velocity_humanize",
    "global.round_robin",
    "mix.output_mode",
];

/// The faders among them: a slider with its value readout beside it.
const GLOBAL_FADERS: [&str; 3] = [
    "global.polyphony",
    "global.velocity_curve",
    "global.velocity_humanize",
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

/// The default tab (Pads) at both sizes.
fn frames() -> Vec<((f32, f32), EditorFrameProbe)> {
    let plugin = ResonanceDrums::new();
    SIZES
        .iter()
        .map(|&size| (size, resonance_drums::test_render_editor_frame(&plugin, size)))
        .collect()
}

/// The Mix tab, where the global controls live, at both sizes.
fn mix_frames() -> Vec<((f32, f32), EditorFrameProbe)> {
    let plugin = ResonanceDrums::new();
    SIZES
        .iter()
        .map(|&size| {
            library::isolate_for_tests();
            let mut editor = TestEditor::new(&plugin, library::shared(), size);
            editor.show_view("Mix");
            editor.frame(Vec::new());
            (size, editor.frame(Vec::new()))
        })
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
    for (size, frame) in mix_frames() {
        for label in GLOBAL_LABELS {
            assert_text_visible(&frame, size, label);
        }
        // The segmented controls' own segments, not just their captions:
        // "Random" was once clipped off the old GLOBAL card's right edge.
        for segment in resonance_drums::params::ROUND_ROBIN_LABELS
            .iter()
            .chain(resonance_drums::params::OUTPUT_MODE_LABELS)
        {
            assert_text_visible(&frame, size, segment);
        }
    }
}

#[test]
fn every_global_control_widget_is_visible_at_both_window_sizes() {
    for (size, frame) in mix_frames() {
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

/// Each fader's value reads to the right of its slider: both visible, no
/// overlap — at the minimum width VELOCITY CURVE's value used to be
/// drawn over its own label.
#[test]
fn global_faders_show_slider_and_value_side_by_side() {
    for (size, frame) in mix_frames() {
        for name in GLOBAL_FADERS {
            let s = frame.widget(name).unwrap();
            let v = frame
                .widget(&format!("{name}.value"))
                .unwrap_or_else(|| panic!("{name}'s value was not probed at {size:?}"));
            for (what, r) in [("slider", s), ("value", v)] {
                if let Err(e) = fully_visible(r.rect, r.clip, frame.screen) {
                    panic!("{name}'s {what} is not visible at {size:?}: {e}");
                }
            }
            assert!(
                s.rect.right() <= v.rect.left() + TOLERANCE,
                "{name}'s value overlaps its slider at {size:?}: {:?}, {:?}",
                s.rect,
                v.rect
            );
        }
    }
}

/// A value readout is never squeezed: each is as wide at the minimum as
/// at the default, i.e. shown in full at both.
#[test]
fn global_values_are_not_elided_at_the_minimum_size() {
    let all = mix_frames();
    let (_, default) = &all[0];
    let (min_size, minimum) = &all[1];
    for name in GLOBAL_FADERS {
        let name = format!("{name}.value");
        let full = default.widget(&name).unwrap().rect.width();
        let small = minimum.widget(&name).unwrap().rect.width();
        assert!(
            (full - small).abs() < 0.5,
            "{name} is elided at {min_size:?}: {small} px wide against {full} px"
        );
    }
}

/// A kit-load error is shown in the header, beside the kit, elided to
/// fit — never painted over the `Library…` button (the KIT card it used
/// to sit in is gone).
#[test]
fn a_long_kit_error_does_not_overrun_the_header() {
    let plugin = ResonanceDrums::new();
    *plugin.bridge.kit_status.lock() = resonance_drums::kit_loader::KitStatus::Error {
        message: "no drum_samples.json found in /a/very/long/path/to/a/kit/that/goes/on/and/on"
            .to_string(),
    };
    for size in SIZES {
        let frame = resonance_drums::test_render_editor_frame(&plugin, size);
        let warning = frame.widget("kit.error").expect("the load error is shown");
        if let Err(e) = fully_visible(warning.rect, warning.clip, frame.screen) {
            panic!("the kit error is not visible at {size:?}: {e}");
        }
        let library = frame.widget("header.library").unwrap();
        assert!(
            library.rect.right() <= warning.rect.left() + TOLERANCE,
            "the error overlaps Library… at {size:?}: {:?} vs {:?}",
            warning.rect,
            library.rect
        );
    }
}

/// The pad grid's cells have real width on screen — the old pad list's
/// rows were once laid out under a 0 px clip while the body ran sideways.
#[test]
fn pad_cells_have_visible_width() {
    for (size, frame) in frames() {
        let cells: Vec<_> = frame
            .widgets
            .iter()
            .filter(|w| w.name.starts_with("pad_cell.") && !w.name.ends_with(".absent"))
            .collect();
        assert_eq!(cells.len(), resonance_drums::drum_map::NUM_PADS, "at {size:?}");
        for cell in cells {
            let visible = cell.rect.intersect(cell.clip).intersect(frame.screen);
            assert!(
                visible.width() > 40.0 && visible.height() > 40.0,
                "{} is not visible at {size:?}: rect {:?}, clip {:?}",
                cell.name,
                cell.rect,
                cell.clip
            );
        }
    }
}

/// The inspector fits beside the pad grid — it used to be laid out
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
