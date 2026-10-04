//! PUX-01: an in-editor preset recall (the bar's ◀/▶ step, a combo
//! pick, a browser commit/audition/import) writes many params at once
//! through `PresetSession`, never through the per-gesture
//! `float_knob`/`param_knob` announce path. Without a rescan the
//! host's mirror of those values goes stale, and `project_plugin`
//! (which only writes an override for a param whose mirror differs
//! from default) can lose the recall on save→reopen.
//!
//! This drives the real `preset_bar` UI — a synthetic click on its ▶
//! button, found from the frame it actually painted — rather than
//! calling `PresetEditor::step` directly, since the thing under test is
//! `preset_ui.rs`'s own wiring, not the GUI-agnostic step logic
//! (covered in `tests/presets.rs`).
#![cfg(feature = "editor-widgets")]

use plugin_gui_core::egui;
use resonance_plugin::editor_widgets::install_announcer;
use resonance_plugin::preset_ui::preset_bar;
use resonance_plugin::presets::{FactoryPreset, PresetBank, PresetEditor, PresetSession};
use resonance_plugin::{EditAnnouncer, FloatParam, FloatRange, Param, PresetEvent};

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
    let path =
        std::env::temp_dir().join(format!("resonance-preset-bar-rescan-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    path
}

/// Every painted text's label and the centre of its bounding rect.
fn text_centers(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Pos2)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
        match shape {
            egui::Shape::Text(t) => {
                out.push((t.galley.text().to_string(), t.visual_bounding_rect().center()))
            }
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

#[test]
fn stepping_through_the_bar_rescans_the_host_mirror_but_announces_nothing() {
    let root = temp_root("step");
    let bank = PresetBank::new("com.resonance.test", FACTORY).with_root(root.clone());
    let mix = FloatParam::new("mix", "Mix", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 });
    let params: Vec<&dyn Param> = vec![&mix];
    let session = PresetSession::new();
    let mut editor = PresetEditor::default();
    let ctx = egui::Context::default();
    let announcer = EditAnnouncer::recording();
    install_announcer(&ctx, &announcer);

    let size = egui::vec2(640.0, 60.0);
    let run = |editor: &mut PresetEditor, events: Vec<egui::Event>| -> (PresetEvent, Vec<egui::epaint::ClippedShape>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        };
        let mut event = PresetEvent::None;
        let out = ctx.run_ui(input, |ui| {
            event = preset_bar(ui, "test_presets", editor, &bank, &session, &params, "— preset —");
        });
        (event, out.shapes)
    };

    // Frame 1: nothing loaded yet. Find the ▶ button (step forward,
    // enabled with nothing loaded — it starts the list from the top).
    let (_, shapes) = run(&mut editor, vec![]);
    let pos = text_centers(&shapes)
        .into_iter()
        .find(|(t, _)| t == "▶")
        .map(|(_, p)| p)
        .expect("the ▶ button painted");

    assert_eq!(session.current(), None, "nothing loaded yet");
    assert_eq!(announcer.rescans_requested(), 0);

    // Click it: move, press, release — the click registers on release.
    run(&mut editor, vec![egui::Event::PointerMoved(pos)]);
    run(
        &mut editor,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    let (event, _) = run(
        &mut editor,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    let loaded = matches!(event, PresetEvent::Loaded(_));
    assert!(loaded, "the ▶ click should load the first preset");

    assert!(session.current().is_some(), "a preset is now loaded");
    assert_eq!(
        announcer.rescans_requested(),
        1,
        "the recall should ask the host to rescan exactly once"
    );
    assert!(
        announcer.announced().is_empty(),
        "a bulk recall is a rescan, not a per-param announce: {:?}",
        announcer.announced()
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn no_recall_means_no_rescan() {
    let root = temp_root("idle");
    let bank = PresetBank::new("com.resonance.test", FACTORY).with_root(root.clone());
    let mix = FloatParam::new("mix", "Mix", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 });
    let params: Vec<&dyn Param> = vec![&mix];
    let session = PresetSession::new();
    let mut editor = PresetEditor::default();
    let ctx = egui::Context::default();
    let announcer = EditAnnouncer::recording();
    install_announcer(&ctx, &announcer);

    let size = egui::vec2(640.0, 60.0);
    for _ in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            preset_bar(ui, "test_presets", &mut editor, &bank, &session, &params, "— preset —");
        });
    }
    assert_eq!(announcer.rescans_requested(), 0);
    let _ = std::fs::remove_dir_all(&root);
}
