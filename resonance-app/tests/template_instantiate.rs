//! Instantiating a template lands a fresh, *untitled* project and never
//! mutates the template source (impl-plan doc #197, todo #665).
//!
//! Two guarantees are proven here:
//! * A built-in starter or a user template replays into app state with the
//!   project path left `None` (so the next Save becomes Save-As) and the
//!   startup modal dismissed (`has_active_project == true`).
//! * Loading a user template off disk leaves the template folder byte-for-byte
//!   unchanged.
//!
//! The instantiate flow is async at the seams (it clears the engine, then
//! replays on the `AllCleared` event). These tests drive it the same way the
//! engine→app mirror tests do: kick the flow, then feed the `AllCleared`
//! event in by hand so the replay runs without a live audio thread.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use resonance_app::project::{load_project, ProjectFile};
use resonance_app::update::project_io::{
    begin_instantiate, instantiate_builtin, write_template, BuiltinTemplateId,
    TemplateCaptureOptions,
};
use resonance_app::Resonance;
use resonance_audio::types::AudioEvent;

/// Recursively read every file under `root` into a sorted map of
/// relative-path → bytes, so two snapshots compare byte-for-byte regardless
/// of directory-walk order.
fn snapshot_dir(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read_dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(root).expect("strip_prefix").to_path_buf();
                out.insert(rel, std::fs::read(&path).expect("read file"));
            }
        }
    }
    out
}

#[test]
fn builtin_lands_a_fresh_untitled_project() {
    let mut app = Resonance::new().0;

    instantiate_builtin(&mut app, BuiltinTemplateId::Beatmaking);
    // The engine confirms the clear; that's what triggers the replay.
    app.test_apply_engine_event(AudioEvent::AllCleared);

    // Untitled: no path (so the next Save is a Save-As), but the project is
    // active (startup modal gone).
    assert_eq!(app.test_project_path(), None);
    assert!(app.test_has_active_project());

    // The Beatmaking starter's content replayed: a drum + bass track, reverb
    // + delay return busses, at 90 BPM.
    let reg = app.test_registry();
    assert_eq!(reg.tracks.len(), 2, "two instrument tracks");
    assert_eq!(reg.busses.len(), 2, "two FX return busses");
    assert_eq!(app.test_transport_bpm(), 90.0);
}

#[test]
fn instantiating_over_an_open_project_clears_the_path() {
    let mut app = Resonance::new().0;
    // Stand in for a project the user had open and saved.
    app.test_set_project_path(Some(PathBuf::from("/tmp/some/existing.rproj")));
    app.test_set_active_project(true);

    instantiate_builtin(&mut app, BuiltinTemplateId::Empty);
    app.test_apply_engine_event(AudioEvent::AllCleared);

    // The prior path is gone: the template instantiation is untitled, so a
    // Save can never overwrite the previously-open project either.
    assert_eq!(app.test_project_path(), None);
    assert!(app.test_has_active_project());
}

#[test]
fn user_template_replays_and_leaves_the_source_untouched() {
    let tmp = tempfile::tempdir().unwrap();

    // A realistic, instantiable project captured as a user template. Built-in
    // starters are designed to round-trip through the project I/O path, so one
    // doubles as a faithful user-template fixture.
    let project: ProjectFile = BuiltinTemplateId::BandRecording.build().file;
    let folder = write_template(
        tmp.path(),
        "My Band Template",
        "captured for testing",
        project,
        &[],
        &[],
        TemplateCaptureOptions::capture_all(),
        1_700_000_000,
    )
    .expect("write_template");

    let before = snapshot_dir(&folder);
    assert!(
        before.contains_key(Path::new("project.json")),
        "template wrote a project.json"
    );

    // Instantiate: load the template off disk and replay it.
    let loaded = load_project(&folder).expect("load_project");
    let mut app = Resonance::new().0;
    begin_instantiate(&mut app, Box::new(loaded));
    app.test_apply_engine_event(AudioEvent::AllCleared);

    // Fresh untitled project, content replayed.
    assert_eq!(app.test_project_path(), None);
    assert!(app.test_has_active_project());
    let reg = app.test_registry();
    assert_eq!(reg.tracks.len(), 6, "six band-recording tracks");
    assert_eq!(reg.busses.len(), 2, "drum + instrument busses");

    // The template folder on disk is byte-for-byte unchanged.
    let after = snapshot_dir(&folder);
    assert_eq!(before, after, "instantiation must not mutate the template");
}

#[test]
fn template_loaded_message_routes_to_a_fresh_untitled_project() {
    use resonance_app::message::{Message, ProjectIoMessage};

    let tmp = tempfile::tempdir().unwrap();
    let project: ProjectFile = BuiltinTemplateId::Beatmaking.build().file;
    let folder = write_template(
        tmp.path(),
        "Routed",
        "",
        project,
        &[],
        &[],
        TemplateCaptureOptions::capture_all(),
        1_700_000_000,
    )
    .expect("write_template");
    let loaded = load_project(&folder).expect("load_project");

    let mut app = Resonance::new().0;
    // Drive the real handler: the async load task maps its result to this
    // message. `ProjectIo` messages pass the startup-modal gate.
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::TemplateLoaded(Ok(
        Box::new(loaded),
    ))));
    app.test_apply_engine_event(AudioEvent::AllCleared);

    assert_eq!(app.test_project_path(), None);
    assert!(app.test_has_active_project());
    assert_eq!(app.test_transport_bpm(), 90.0);
}
