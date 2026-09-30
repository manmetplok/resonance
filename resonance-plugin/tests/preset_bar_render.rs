//! Headless render smoke test for the shared preset bar.
//!
//! The bar's behaviour is covered in `tests/presets.rs` against the
//! GUI-agnostic `PresetEditor`; what is left to prove here is that the
//! egui skin actually lays out — id collisions, borrow conflicts between
//! the name buffer and the surrounding `Ui`, and enabled/disabled button
//! states are all frame-time failures no unit test would see.
//!
//! Runs with `--features editor-widgets` (empty otherwise), and needs no
//! display: `egui::__run_test_ui` drives a full frame on the CPU.
#![cfg(feature = "editor-widgets")]

use resonance_plugin::preset_ui::preset_bar;
use resonance_plugin::presets::{FactoryPreset, PresetBank, PresetEditor, PresetRef, PresetSession};
use resonance_plugin::{FloatParam, FloatRange, Param, PresetEvent};
use plugin_gui_core::egui;

const FACTORY: &[FactoryPreset] = &[
    FactoryPreset {
        id: "init",
        name: "Init",
        json: r#"{"params":{"mix":0.5}}"#,
    },
    FactoryPreset {
        id: "wide",
        name: "Wide",
        json: r#"{"params":{"mix":0.9}}"#,
    },
];

fn temp_root(tag: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("resonance-preset-bar-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    path
}

#[test]
fn the_bar_lays_out_in_every_state_it_can_be_in() {
    let root = temp_root("render");
    let bank = PresetBank::new("com.resonance.test", FACTORY).with_root(root.clone());
    let mix = FloatParam::new("mix", "Mix", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 });
    let params: Vec<&dyn Param> = vec![&mix];
    let session = PresetSession::new();
    let mut editor = PresetEditor::default();

    // 1. Nothing loaded, no user presets yet.
    egui::__run_test_ui(|ui| {
        let event = preset_bar(
            ui,
            "test_presets",
            &mut editor,
            &bank,
            &session,
            &params,
            "— preset —",
        );
        assert_eq!(event, PresetEvent::None);
    });

    // 2. A factory preset loaded and edited since (the "•" branch), with
    //    a user preset in the list so both headings render.
    let mine = session.save_as(&bank, "Mine", &params).unwrap();
    session.set_current(Some(PresetRef::factory("init", "Init")));
    session.mark_modified();
    egui::__run_test_ui(|ui| {
        preset_bar(
            ui,
            "test_presets",
            &mut editor,
            &bank,
            &session,
            &params,
            "— preset —",
        );
    });

    // 3. Mid-rename: the name field replaces the buttons.
    editor.begin_rename(&mine);
    egui::__run_test_ui(|ui| {
        preset_bar(
            ui,
            "test_presets",
            &mut editor,
            &bank,
            &session,
            &params,
            "— preset —",
        );
    });
    assert!(editor.naming().is_some(), "no click happened, so no submit");

    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// The browser overlay and the metadata form, read back from the frame's
// text shapes (plugin-preset-library.md P4)
// ---------------------------------------------------------------------------

/// Every text the frame painted.
fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in shapes {
        walk(&s.shape, &mut out);
    }
    out
}

fn has(drawn: &[String], needle: &str) -> bool {
    drawn.iter().any(|t| t.contains(needle))
}

/// One frame of the bar (and whatever overlay it opens) in a window of
/// `size`.
fn frame(
    ctx: &egui::Context,
    size: egui::Vec2,
    editor: &mut PresetEditor,
    bank: &PresetBank,
    session: &PresetSession,
    params: &[&dyn Param],
) -> Vec<String> {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        ..Default::default()
    };
    let out = ctx.run_ui(input, |ui| {
        preset_bar(ui, "test_presets", editor, bank, session, params, "— preset —");
    });
    texts(&out.shapes)
}

/// The browser at a wide editor and at the narrowest editor in the fleet
/// (the gate's 640×260 minimum): the list, the loaded preset's detail
/// pane and the footer all draw, in both layouts.
#[test]
fn the_browser_draws_its_list_detail_and_footer_wide_and_narrow() {
    for (tag, size) in [("wide", egui::vec2(1000.0, 600.0)), ("narrow", egui::vec2(640.0, 260.0))] {
        let root = temp_root(&format!("browser-{tag}"));
        let bank = PresetBank::new("com.resonance.test", FACTORY)
            .with_root(root.clone())
            .with_plugin_info("Test Plugin", "1.0.0");
        let mix = FloatParam::new("mix", "Mix", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 });
        let params: Vec<&dyn Param> = vec![&mix];
        let session = PresetSession::new();
        session.save_as(&bank, "My Keeper", &params).unwrap();
        let mut editor = PresetEditor::default();
        let ctx = egui::Context::default();

        let bar = frame(&ctx, size, &mut editor, &bank, &session, &params);
        for label in ["Browse", "Save as…", "My Keeper"] {
            assert!(has(&bar, label), "{tag}: the bar draws {label:?}: {bar:?}");
        }
        editor.browser.open(&bank, &session);
        frame(&ctx, size, &mut editor, &bank, &session, &params); // an area's first frame measures
        let drawn = frame(&ctx, size, &mut editor, &bank, &session, &params);
        assert!(editor.browser.open, "{tag}: no click, so it stays open");
        for label in ["Presets", "· Test Plugin", "Init", "Wide", "My Keeper", "Import…", "Enter keep"] {
            assert!(has(&drawn, label), "{tag}: the browser draws {label:?}: {drawn:?}");
        }
        assert!(has(&drawn, "3 presets"), "{tag}: {drawn:?}");
        // The detail pane names the selected (loaded) preset and its actions.
        for label in ["user", "Duplicate", "Edit info…"] {
            assert!(has(&drawn, label), "{tag}: the detail pane draws {label:?}: {drawn:?}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// The metadata form (Save as…) draws its fields, pre-filled.
#[test]
fn the_metadata_form_draws_its_fields() {
    let root = temp_root("form");
    let bank = PresetBank::new("com.resonance.test", FACTORY).with_root(root.clone());
    let mix = FloatParam::new("mix", "Mix", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 });
    let params: Vec<&dyn Param> = vec![&mix];
    let session = PresetSession::new();
    session.set_current(Some(PresetRef::factory("init", "Init")));
    let mut editor = PresetEditor::default();
    let ctx = egui::Context::default();
    editor.browser.begin_save_as(&bank, &session);
    let size = egui::vec2(640.0, 260.0);
    frame(&ctx, size, &mut editor, &bank, &session, &params);
    let drawn = frame(&ctx, size, &mut editor, &bank, &session, &params);
    for label in ["Save preset", "Name", "Category", "For", "Genres", "Character", "Save", "Cancel"] {
        assert!(has(&drawn, label), "the form draws {label:?}: {drawn:?}");
    }
    assert!(has(&drawn, "Init (edit)"), "the name is pre-filled: {drawn:?}");
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// Keys and clipping (review M8, narrow layout)
// ---------------------------------------------------------------------------

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// One frame with `events`, returning the painted shapes.
fn frame_with(
    ctx: &egui::Context,
    size: egui::Vec2,
    events: Vec<egui::Event>,
    editor: &mut PresetEditor,
    bank: &PresetBank,
    session: &PresetSession,
    params: &[&dyn Param],
) -> Vec<egui::epaint::ClippedShape> {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        events,
        ..Default::default()
    };
    ctx.run_ui(input, |ui| {
        preset_bar(ui, "test_presets", editor, bank, session, params, "— preset —");
    })
    .shapes
}

/// Every painted text whose rect leaves its clip rect (or the window).
fn clipped_texts(shapes: &[egui::epaint::ClippedShape], size: egui::Vec2) -> Vec<String> {
    fn walk(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => {
                let rect = t.galley.rect.translate(t.pos.to_vec2());
                // Horizontally, a text must fit its clip; vertically a scroll
                // area legitimately cuts content off, so only a text that
                // shows at all is checked, and only across.
                let shows = rect.max.y > clip.min.y && rect.min.y < clip.max.y;
                let fits = rect.min.x >= clip.min.x - 0.5 && rect.max.x <= clip.max.x + 0.5;
                if !t.galley.text().trim().is_empty() && shows && !fits {
                    out.push(format!("{:?} at {rect:?} outside {clip:?}", t.galley.text()));
                }
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            _ => {}
        }
    }
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let mut out = Vec::new();
    for s in shapes {
        walk(&s.shape, s.clip_rect.intersect(window), &mut out);
    }
    out
}

/// With the metadata form open over the browser, ↓ does not audition the
/// list behind it and Esc closes only the form; with a rename open, Esc
/// cancels only the rename.
#[test]
fn keys_belong_to_the_form_or_rename_in_front() {
    let root = temp_root("keys");
    let bank = PresetBank::new("com.resonance.test", FACTORY).with_root(root.clone());
    let mix = FloatParam::new("mix", "Mix", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 });
    let params: Vec<&dyn Param> = vec![&mix];
    let session = PresetSession::new();
    session.save_as(&bank, "Mine", &params).unwrap();
    let mut editor = PresetEditor::default();
    let ctx = egui::Context::default();
    let size = egui::vec2(1000.0, 600.0);
    editor.browser.open(&bank, &session);
    frame_with(&ctx, size, vec![], &mut editor, &bank, &session, &params);
    let before = session.current();

    editor.browser.begin_save_as(&bank, &session);
    frame_with(&ctx, size, vec![], &mut editor, &bank, &session, &params);
    frame_with(&ctx, size, vec![key(egui::Key::ArrowDown)], &mut editor, &bank, &session, &params);
    assert_eq!(session.current(), before, "↓ under the form auditions nothing");
    frame_with(&ctx, size, vec![key(egui::Key::Escape)], &mut editor, &bank, &session, &params);
    assert!(editor.browser.form.is_none(), "Esc closes the form");
    assert!(editor.browser.open, "and only the form");

    let selected = editor.browser.model.selected().map(str::to_string).unwrap();
    editor.browser.begin_rename(&selected);
    frame_with(&ctx, size, vec![], &mut editor, &bank, &session, &params);
    frame_with(&ctx, size, vec![key(egui::Key::Escape)], &mut editor, &bank, &session, &params);
    assert!(editor.browser.rename.is_none(), "Esc cancels the rename");
    assert!(editor.browser.open, "and leaves the browser open");
    let _ = std::fs::remove_dir_all(&root);
}

/// At the fleet's smallest editor (640×260) every text the browser, its
/// detail pane and the form paint lies inside its clip rect.
#[test]
fn nothing_is_clipped_at_the_minimum_editor_size() {
    let root = temp_root("clip");
    let bank = PresetBank::new("com.resonance.test", FACTORY)
        .with_root(root.clone())
        .with_plugin_info("Test Plugin", "1.0.0");
    let mix = FloatParam::new("mix", "Mix", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 });
    let params: Vec<&dyn Param> = vec![&mix];
    let session = PresetSession::new();
    session.save_as(&bank, "My Keeper", &params).unwrap();
    let mut editor = PresetEditor::default();
    let ctx = egui::Context::default();
    let size = egui::vec2(640.0, 260.0);
    editor.browser.open(&bank, &session);
    frame_with(&ctx, size, vec![], &mut editor, &bank, &session, &params);
    let shapes = frame_with(&ctx, size, vec![], &mut editor, &bank, &session, &params);
    let clipped = clipped_texts(&shapes, size);
    assert!(clipped.is_empty(), "browser: {clipped:#?}");

    editor.browser.begin_save_as(&bank, &session);
    frame_with(&ctx, size, vec![], &mut editor, &bank, &session, &params);
    let shapes = frame_with(&ctx, size, vec![], &mut editor, &bank, &session, &params);
    let clipped = clipped_texts(&shapes, size);
    assert!(clipped.is_empty(), "form: {clipped:#?}");
    let _ = std::fs::remove_dir_all(&root);
}
