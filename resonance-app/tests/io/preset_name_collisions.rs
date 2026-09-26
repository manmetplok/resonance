//! Distinct preset names never share a file (code review STATE-15).
//!
//! The old filename sanitizer mapped every non-alphanumeric character to
//! `_`, so "A B", "A.B" and "A_B" were all `A_B.json`: saving one silently
//! replaced another, and deleting one removed the other. Names now encode
//! injectively, a save refuses to replace a file holding a different
//! preset, and delete matches the stored name.
//!
//! Runs against the hermetic per-process preset dir a test app sets up
//! (STATE-14), so each test uses names of its own.

use std::path::PathBuf;

use resonance_app::presets::{self, TrackPreset};
use resonance_app::Resonance;

fn preset(name: &str) -> TrackPreset {
    TrackPreset {
        name: name.into(),
        track_type: "audio".into(),
        volume: 0.0,
        pan: 0.0,
        mono: false,
        instrument_type: Default::default(),
        instrument_icon: Default::default(),
        role: None,
        plugins: Vec::new(),
    }
}

fn hermetic() {
    let _ = Resonance::new_for_test();
}

fn loaded(name: &str) -> bool {
    presets::load_user_presets().iter().any(|p| p.name == name)
}

#[test]
fn names_differing_only_in_punctuation_both_survive() {
    hermetic();
    let a = "STATE-15 collide A B";
    let b = "STATE-15 collide A.B";
    let c = "STATE-15 collide A_B";
    let pa = presets::save_user_preset(&preset(a)).unwrap();
    assert!(!presets::user_preset_exists(b), "{b:?} must not look taken by {a:?}");
    let pb = presets::save_user_preset(&preset(b)).unwrap();
    let pc = presets::save_user_preset(&preset(c)).unwrap();
    assert!(pa != pb && pb != pc && pa != pc);
    assert!(loaded(a) && loaded(b) && loaded(c), "all three presets load");

    presets::delete_user_preset(b).unwrap();
    assert!(!loaded(b));
    assert!(loaded(a) && loaded(c), "deleting {b:?} must not remove the others");
}

#[test]
fn empty_and_dot_names_are_handled() {
    hermetic();
    assert!(presets::save_user_preset(&preset("")).is_err(), "empty name refused");
    assert!(presets::save_user_preset(&preset("   ")).is_err(), "blank name refused");
    for name in [".", "..", "/", "../escape"] {
        let path = presets::save_user_preset(&preset(name)).unwrap();
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(!file.starts_with('.'), "{name:?} became hidden file {file:?}");
        assert!(!file.contains('/'));
        assert!(loaded(name));
        presets::delete_user_preset(name).unwrap();
        assert!(!path.exists());
    }
}

#[test]
fn a_file_holding_another_preset_is_never_overwritten() {
    hermetic();
    let name = "STATE-15 guard";
    let path = presets::save_user_preset(&preset(name)).unwrap();
    // Some other preset occupies the file this name maps to (a case-
    // insensitive filesystem, or a hand-copied file).
    let other = serde_json::to_string(&preset("STATE-15 someone else")).unwrap();
    std::fs::write(&path, other).unwrap();
    assert!(presets::save_user_preset(&preset(name)).is_err());
    assert!(loaded("STATE-15 someone else"), "the other preset survives");
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn a_preset_saved_under_the_old_filename_is_found_by_its_name() {
    hermetic();
    let name = "STATE-15 legacy X";
    let dir: PathBuf = presets::save_user_preset(&preset("STATE-15 legacy probe"))
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let legacy = dir.join("STATE-15_legacy_X.json");
    std::fs::write(&legacy, serde_json::to_string(&preset(name)).unwrap()).unwrap();

    assert!(presets::user_preset_exists(name));
    // Overwriting it replaces the legacy file instead of adding a twin.
    presets::save_user_preset(&preset(name)).unwrap();
    assert_eq!(
        presets::load_user_presets().iter().filter(|p| p.name == name).count(),
        1
    );
    presets::delete_user_preset(name).unwrap();
    assert!(!loaded(name));
    assert!(!legacy.exists());
}
