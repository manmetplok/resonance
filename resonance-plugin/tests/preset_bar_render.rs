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
        name: "Init",
        json: r#"{"params":{"mix":0.5}}"#,
    },
    FactoryPreset {
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
    session.save_as(&bank, "Mine", &params).unwrap();
    session.set_current(Some(PresetRef::factory("Init")));
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
    editor.begin_rename(&PresetRef::user("Mine"));
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
