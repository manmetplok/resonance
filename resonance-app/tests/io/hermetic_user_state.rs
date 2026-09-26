//! A test-built app never touches the developer's real per-user state
//! (code review STATE-14 / FU-D5).
//!
//! `new_for_test*` skipped *reading* recents, settings and presets, but a
//! completed save or load still wrote `~/.config/resonance/recent.json`
//! (tempfile paths turned up in the real recent-projects list), and the
//! preset prompt's "Save"/"Overwrite" label read the real preset folder.
//! Building a test app now makes the process hermetic: all of it resolves
//! under a per-process temp root.

use std::path::{Path, PathBuf};

use resonance_app::message::{Message, ProjectIoMessage};
use resonance_app::presets::{self, TrackPreset};
use resonance_app::{user_dirs, Resonance};

fn unique_project(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "resonance-hermetic-{tag}-{}/song.rproj",
        std::process::id()
    ))
}

fn file_mentions(file: &Path, needle: &Path) -> bool {
    std::fs::read_to_string(file)
        .map(|s| s.contains(needle.to_string_lossy().as_ref()))
        .unwrap_or(false)
}

#[test]
fn a_completed_save_records_the_recent_under_the_hermetic_root() {
    let (mut app, _task) = Resonance::new_for_test();
    let root = user_dirs::hermetic_root().expect("a test app makes the process hermetic");
    assert!(root.starts_with(std::env::temp_dir()));

    let project = unique_project("recent");
    app.test_set_project_path(project.clone());
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectSaved(Ok(()), false)));

    let hermetic = user_dirs::config_dir().unwrap().join("resonance/recent.json");
    assert!(hermetic.starts_with(root));
    assert!(
        file_mentions(&hermetic, &project),
        "the save's recent entry lands in the hermetic config dir"
    );
    // Read-only check of the real file: the entry must not be there.
    if let Some(real) = dirs::config_dir().map(|d| d.join("resonance/recent.json")) {
        assert!(!file_mentions(&real, &project), "the real recent.json was written");
    }
}

#[test]
fn user_presets_resolve_under_the_hermetic_root() {
    let _ = Resonance::new_for_test();
    let root = user_dirs::hermetic_root().unwrap();
    if std::env::var_os(presets::PRESET_DIR_ENV).is_some() {
        return; // an explicit override wins, and is hermetic by itself
    }
    let preset = TrackPreset {
        name: "Hermetic probe preset".into(),
        track_type: "audio".into(),
        volume: 0.0,
        pan: 0.0,
        mono: false,
        instrument_type: Default::default(),
        instrument_icon: Default::default(),
        role: None,
        plugins: Vec::new(),
    };
    let path = presets::save_user_preset(&preset).expect("save");
    assert!(path.starts_with(root), "{} is outside the hermetic root", path.display());
}
