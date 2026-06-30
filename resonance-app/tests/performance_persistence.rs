//! Persistence coverage for the Performance-mode footer selection
//! (epic #11, todo #312, design #151): the live fingering diagrams'
//! instrument tuning and capo offset. Proves the selection serializes into
//! the on-disk `ProjectFile`, restores back through the same path a project
//! load runs, and round-trips byte-for-byte — while a legacy project that
//! predates the `performance` field loads cleanly with the default
//! Guitar 6 / no-capo selection.

use resonance_app::project::ProjectFile;
use resonance_app::Resonance;
use resonance_music_theory::{BASS_5, GUITAR_6, ALL_TUNINGS};

/// Index of a deliberately non-default tuning so a round-trip that silently
/// dropped the selection (and fell back to Guitar 6 at index 0) would be
/// caught. Bass (5-string) is the last entry in `ALL_TUNINGS`.
fn bass5_index() -> usize {
    ALL_TUNINGS
        .iter()
        .position(|t| t.name == BASS_5.name)
        .expect("BASS_5 is in ALL_TUNINGS")
}

#[test]
fn build_project_file_captures_tuning_and_capo() {
    let (mut app, _task) = Resonance::new();

    {
        let p = app.test_performance_mut();
        p.set_tuning_index(bass5_index());
        p.set_capo(4);
    }

    let file = app.test_build_project_file();

    assert_eq!(file.performance.tuning, BASS_5.name);
    assert_eq!(file.performance.capo, 4);
}

#[test]
fn tuning_and_capo_round_trip_identically() {
    // -- Author a project with a non-default tuning + capo -----------
    let (mut authored, _t1) = Resonance::new();
    {
        let p = authored.test_performance_mut();
        p.set_tuning_index(bass5_index());
        p.set_capo(7);
    }

    let saved = authored.test_build_project_file();

    // Persist through real JSON, exactly as a save/reload would.
    let json = serde_json::to_string(&saved).expect("serialize project");
    let reloaded: ProjectFile = serde_json::from_str(&json).expect("deserialize project");

    // -- Restore into a fresh app via the production restore path -----
    let (mut restored, _t2) = Resonance::new();
    restored.test_restore_performance(&reloaded);

    let p = restored.test_performance();
    assert_eq!(p.tuning().name, BASS_5.name, "tuning resolved by name");
    assert_eq!(p.tuning_index, bass5_index());
    assert_eq!(p.capo, 7, "capo restored");

    // Re-serializing the restored app yields byte-identical JSON — the
    // definition of a clean round-trip.
    let re_saved = restored.test_build_project_file();
    let re_json = serde_json::to_string(&re_saved).expect("re-serialize");
    assert_eq!(re_json, json, "performance selection round-trips identically");
}

#[test]
fn unknown_tuning_name_falls_back_to_default() {
    // A project hand-edited (or saved by a future build) to reference a
    // tuning this build doesn't know must resolve to the default Guitar 6
    // rather than panic or desync the diagram renderer.
    let mut saved = ProjectFile::default();
    saved.performance.tuning = "Sitar (19-string)".to_string();
    saved.performance.capo = 2;

    let (mut app, _task) = Resonance::new();
    app.test_restore_performance(&saved);

    let p = app.test_performance();
    assert_eq!(p.tuning().name, GUITAR_6.name, "unknown tuning -> default");
    assert_eq!(p.tuning_index, 0);
    // Capo is independent of the tuning name and still restores.
    assert_eq!(p.capo, 2);
}

#[test]
fn legacy_project_without_performance_loads_clean() {
    // A project authored before the `performance` field existed has no
    // such key. `#[serde(default)]` must fill it with the default
    // Guitar 6 / no-capo selection without error.
    let mut json = serde_json::to_value(ProjectFile::default()).unwrap();
    let obj = json.as_object_mut().unwrap();
    obj.remove("performance");

    let file: ProjectFile =
        serde_json::from_value(json).expect("legacy project deserializes");
    assert_eq!(file.performance.tuning, GUITAR_6.name);
    assert_eq!(file.performance.capo, 0);

    // Restoring the default legacy block leaves a fresh app on Guitar 6.
    let (mut app, _task) = Resonance::new();
    app.test_restore_performance(&file);
    assert_eq!(app.test_performance().tuning().name, GUITAR_6.name);
    assert_eq!(app.test_performance().tuning_index, 0);
    assert_eq!(app.test_performance().capo, 0);
}
