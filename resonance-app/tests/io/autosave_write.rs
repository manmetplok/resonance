//! Coverage for the autosave write path (todo #463, doc #171 "Autosave
//! triggering"). Two concerns are pinned here:
//!
//!   1. **Serialization.** `project::save_autosave` writes the project
//!      metadata to `project.autosave.json` — a *side file* that never
//!      overwrites the canonical `project.json`, so a manual save and an
//!      autosave can coexist in the same `.rproj` without clobbering
//!      each other, and the snapshot round-trips through serde.
//!   2. **Routing.** The shared `ProjectSaved` completion path branches
//!      on the autosave flag: an autosave records `last_autosave_at`,
//!      leaves `dirty` set, and never touches the recents list or
//!      `last_saved_at`; a manual save does the opposite. A never-saved
//!      project autosaves into a per-session scratch dir.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use resonance_app::message::{Message, ProjectIoMessage, TransportMessage};
use resonance_app::project::{self, ProjectFile, AUTOSAVE_JSON, PROJECT_JSON};
use resonance_app::Resonance;

/// A unique temp directory that deletes itself when dropped. Mirrors the
/// helper in `project_atomic_write.rs` (no `tempfile` dependency in this
/// crate) so each test stays isolated and leaves no litter behind.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "resonance_autosave_{tag}_{}_{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---- Serialization ---------------------------------------------------

#[test]
fn autosave_writes_side_file_not_project_json() {
    let dir = TempDir::new("sidefile");

    let project = ProjectFile {
        bpm: 137.0,
        ..ProjectFile::default()
    };
    project::save_autosave(dir.path(), &project, &[], &[]).expect("save autosave");

    let autosave_path = dir.path().join(AUTOSAVE_JSON);
    assert!(
        autosave_path.exists(),
        "autosave must write {AUTOSAVE_JSON}"
    );
    assert!(
        !dir.path().join(PROJECT_JSON).exists(),
        "autosave must NOT write or overwrite {PROJECT_JSON}"
    );

    // The snapshot round-trips through serde.
    let json = std::fs::read_to_string(&autosave_path).expect("read snapshot");
    let restored: ProjectFile = serde_json::from_str(&json).expect("parse snapshot");
    assert_eq!(restored.bpm, 137.0);
}

#[test]
fn autosave_and_manual_save_coexist_without_clobbering() {
    let dir = TempDir::new("coexist");

    // A committed manual save and a later, diverged autosave snapshot.
    let committed = ProjectFile {
        bpm: 100.0,
        ..ProjectFile::default()
    };
    let snapshot = ProjectFile {
        bpm: 200.0,
        ..ProjectFile::default()
    };
    project::save_project(dir.path(), &committed, &[], &[]).expect("manual save");
    project::save_autosave(dir.path(), &snapshot, &[], &[]).expect("autosave");

    // Both files exist and carry their own, independent contents.
    let loaded = project::load_project(dir.path()).expect("load project.json");
    assert_eq!(
        loaded.file.bpm, 100.0,
        "project.json keeps the manually-saved value"
    );

    let autosave_json =
        std::fs::read_to_string(dir.path().join(AUTOSAVE_JSON)).expect("read autosave");
    let autosave: ProjectFile = serde_json::from_str(&autosave_json).expect("parse autosave");
    assert_eq!(
        autosave.bpm, 200.0,
        "project.autosave.json keeps the snapshot value"
    );
}

// ---- Completion routing ----------------------------------------------

/// Point `dirs::config_dir()` at a throwaway directory for the whole test
/// binary. Booting `Resonance` loads — and a completed save rewrites — the
/// real `~/.config/resonance/recent.json`; besides polluting the user's
/// recents with temp paths, a real list already at the MAX_RECENT cap makes
/// count-based assertions here fail. Set once before any threads read the
/// environment; every test that constructs `Resonance` must call this first.
fn isolate_user_config() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!(
            "resonance_autosave_config_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create isolated config dir");
        std::env::set_var("XDG_CONFIG_HOME", &dir);
    });
}

fn dispatch(app: &mut Resonance, m: ProjectIoMessage) {
    let _ = app.update(Message::ProjectIo(m));
}

#[test]
fn autosave_completion_keeps_dirty_and_records_autosave_time() {
    isolate_user_config();
    let (mut app, _task) = Resonance::new_for_test();

    // Mark the session as an active project so interactive edits aren't
    // gated, then make one edit to dirty it. (`ProjectSaved` is never
    // gated, so the manual completion below always processes.)
    dispatch(&mut app, ProjectIoMessage::ProjectSaved(Ok(()), false));
    let manual_saved_at = app.last_saved_at();
    assert!(manual_saved_at.is_some(), "manual save recorded a time");

    let _ = app.update(Message::Transport(TransportMessage::ToggleMetronome));
    assert!(app.is_dirty(), "an edit must dirty the project");

    // Recents may be pre-populated from disk; pin the count so we can
    // assert the autosave leaves it untouched.
    let recents_before = app.recent_project_count();

    // Autosave completes.
    dispatch(&mut app, ProjectIoMessage::ProjectSaved(Ok(()), true));

    assert!(app.is_dirty(), "autosave must NOT clear the dirty flag");
    assert!(
        app.last_autosave_at().is_some(),
        "autosave records last_autosave_at"
    );
    assert_eq!(
        app.last_saved_at(),
        manual_saved_at,
        "autosave must not touch last_saved_at"
    );
    assert_eq!(
        app.recent_project_count(),
        recents_before,
        "autosave must not touch the recents list"
    );
    assert!(!app.is_saving(), "saving flag cleared on completion");
}

#[test]
fn manual_save_completion_clears_dirty_and_records_save_time() {
    isolate_user_config();
    let (mut app, _task) = Resonance::new_for_test();
    assert!(!app.is_dirty());

    dispatch(&mut app, ProjectIoMessage::ProjectSaved(Ok(()), false));

    assert!(!app.is_dirty(), "manual save clears the dirty flag");
    assert!(app.last_saved_at().is_some(), "records last_saved_at");
    assert!(
        app.last_autosave_at().is_none(),
        "manual save leaves last_autosave_at untouched"
    );
    assert!(!app.is_saving(), "saving flag cleared on completion");
}

#[test]
fn manual_save_with_path_adds_to_recents() {
    isolate_user_config();
    let (mut app, _task) = Resonance::new_for_test();
    let dir = TempDir::new("recents");
    let project = dir.path().join("MyProject");
    let recents_before = app.recent_project_count();

    // SavePathSelected sets the project path and kicks off a save (which
    // sets `saving` and creates the `.rproj` directory).
    dispatch(
        &mut app,
        ProjectIoMessage::SavePathSelected(Some(project.to_string_lossy().into_owned())),
    );
    assert!(app.is_saving(), "selecting a path kicks off a save");
    assert_eq!(
        app.recent_project_count(),
        recents_before,
        "not recent until the save completes"
    );

    // The engine reports both branches, then the async write completes.
    use resonance_audio::types::AudioEvent;
    app.test_apply_engine_event(AudioEvent::ClipsSavedToProjectDir { clip_files: Vec::new() });
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved { states: Vec::new() });
    dispatch(&mut app, ProjectIoMessage::ProjectSaved(Ok(()), false));

    assert_eq!(
        app.recent_project_count(),
        recents_before + 1,
        "a completed manual save lands in recents"
    );
    assert!(!app.is_saving());

    // The save created `{project}.rproj`; clean it up.
    let _ = std::fs::remove_dir_all(project.with_extension("rproj"));
}

#[test]
fn autosave_of_never_saved_project_targets_a_scratch_dir() {
    isolate_user_config();
    let (mut app, _task) = Resonance::new_for_test();
    assert!(app.last_autosave_at().is_none());

    // No project path set → the autosave snapshots into a per-session
    // scratch dir under the app-data directory. The write itself runs
    // async on the engine path, but the directory is created synchronously
    // when the save kicks off.
    dispatch(&mut app, ProjectIoMessage::Autosave);
    assert!(
        app.is_saving(),
        "autosave kicked off even with no project path"
    );

    // A test app is hermetic: the scratch dir lives under the process's
    // temp root, never the developer's real app-data dir (FU-M12b).
    let scratch = resonance_app::user_dirs::hermetic_root()
        .expect("test apps are hermetic")
        .join("data")
        .join("resonance")
        .join("autosave")
        .join(app.session_id());
    assert!(
        scratch.exists(),
        "never-saved autosave creates its scratch dir: {}",
        scratch.display()
    );

    let _ = std::fs::remove_dir_all(&scratch);
}

// ---- Autosave never touches the canonical project (STATE-11) ----------

fn midi_clip(id: u64) -> project::ProjectMidiClip {
    project::ProjectMidiClip {
        id,
        track_id: 1,
        start_sample: 0,
        duration_ticks: 960,
        name: "clip".into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
        midi_file: format!("midi/clip_{id}.mid"),
        vocal_lyrics: Vec::new(),
        notes: None,
    }
}

fn plugin(instance_id: u64) -> project::ProjectPlugin {
    serde_json::from_value(serde_json::json!({
        "instance_id": instance_id,
        "plugin_name": "Synth",
        "clap_plugin_id": "com.example.synth",
        "clap_file_path": "/nowhere/synth.clap",
        "state_file": format!("plugins/plugin_{instance_id}.bin"),
    }))
    .expect("plugin json")
}

fn note(pitch: u8) -> resonance_audio::types::MidiNote {
    resonance_audio::types::MidiNote {
        note: pitch,
        velocity: 0.8,
        start_tick: 0,
        duration_ticks: 480,
    }
}

/// Code review STATE-11 (1): the autosave wrote `midi/clip_*.mid` and
/// `plugins/plugin_*.bin` into the project folder, overwriting the files
/// the saved `project.json` points to. Quitting with "Don't save" then
/// reopened the old arrangement with the unsaved notes and plugin states.
#[test]
fn autosave_leaves_the_canonical_midi_and_plugin_files_alone() {
    let dir = TempDir::new("canonical");
    let file = ProjectFile {
        midi_clips: vec![midi_clip(1)],
        master_plugins: vec![plugin(5)],
        ..ProjectFile::default()
    };
    let saved_state = b"saved state".to_vec();
    project::save_project(dir.path(), &file, &[(5, saved_state.clone())], &[(1, vec![note(60)])])
        .expect("manual save");
    let mid = std::fs::read(dir.path().join("midi/clip_1.mid")).expect("canonical mid");

    let unsaved_state = b"unsaved state".to_vec();
    project::save_autosave(dir.path(), &file, &[(5, unsaved_state.clone())], &[(1, vec![note(72)])])
        .expect("autosave");

    assert_eq!(std::fs::read(dir.path().join("midi/clip_1.mid")).unwrap(), mid);
    assert_eq!(std::fs::read(dir.path().join("plugins/plugin_5.bin")).unwrap(), saved_state);
    let reopened = project::load_project(dir.path()).expect("reopen");
    assert_eq!(&reopened.plugin_states[&5][..], &saved_state[..]);
    assert_eq!(reopened.midi_notes[&1][0].note, 60);

    // The snapshot is complete on its own: its side files live in the
    // autosave subtree and its JSON points there.
    let snap: ProjectFile = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join(AUTOSAVE_JSON)).unwrap(),
    )
    .unwrap();
    let state_file = &snap.master_plugins[0].state_file;
    assert!(state_file.starts_with("autosave/"), "{state_file}");
    assert_eq!(std::fs::read(dir.path().join(state_file)).unwrap(), unsaved_state);
    let midi_file = &snap.midi_clips[0].midi_file;
    assert!(midi_file.starts_with("autosave/"), "{midi_file}");
    assert!(dir.path().join(midi_file).exists());
}

/// Code review STATE-11 (2): a manual save requested while an autosave's
/// engine round-trip is in flight replaced the autosave's collector, so
/// the autosave's `ClipsSavedToProjectDir` / `AllPluginStatesSaved` (for
/// the scratch dir) completed the manual save. The manual save now waits
/// and runs its own round-trip once the autosave has collected.
#[test]
fn a_manual_save_during_an_autosave_runs_its_own_engine_round_trip() {
    use resonance_audio::types::{AudioCommand, AudioEvent};

    isolate_user_config();
    let (mut app, _task, cmds) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    let dir = TempDir::new("interrupt");
    let saved = dir.path().join("song.rproj");
    app.test_set_project_path(saved.clone());
    let _ = app.update(Message::Transport(TransportMessage::ToggleMetronome));

    dispatch(&mut app, ProjectIoMessage::Autosave);
    dispatch(&mut app, ProjectIoMessage::SaveProject);
    // The autosave's two engine reports.
    app.test_apply_engine_event(AudioEvent::ClipsSavedToProjectDir { clip_files: Vec::new() });
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved { states: Vec::new() });

    assert_eq!(
        app.test_save_in_flight(),
        Some((saved.clone(), false)),
        "the manual save is still collecting its own results"
    );
    let clip_saves = cmds
        .try_iter()
        .filter(|c| matches!(c, AudioCommand::SaveClipsToProjectDir))
        .count();
    assert_eq!(clip_saves, 2, "one engine round-trip per save");

    app.test_apply_engine_event(AudioEvent::ClipsSavedToProjectDir { clip_files: Vec::new() });
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved { states: Vec::new() });
    assert_eq!(app.test_save_in_flight(), None);

    // The autosave's completion lands while nothing else is in flight.
    dispatch(&mut app, ProjectIoMessage::ProjectSaved(Ok(()), true));
    assert!(app.is_dirty(), "an autosave never cleans");
}

/// The autosave's completion must not wipe a manual save's collector
/// that started after the autosave captured.
#[test]
fn an_autosave_completion_does_not_drop_a_running_manual_save() {
    use resonance_audio::types::AudioEvent;

    isolate_user_config();
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    let dir = TempDir::new("completion");
    let saved = dir.path().join("song.rproj");
    app.test_set_project_path(saved.clone());

    dispatch(&mut app, ProjectIoMessage::Autosave);
    app.test_apply_engine_event(AudioEvent::ClipsSavedToProjectDir { clip_files: Vec::new() });
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved { states: Vec::new() });
    // The autosave is writing; the user saves.
    dispatch(&mut app, ProjectIoMessage::SaveProject);
    dispatch(&mut app, ProjectIoMessage::ProjectSaved(Ok(()), true));

    assert_eq!(app.test_save_in_flight(), Some((saved, false)));
    assert!(app.is_saving(), "the manual save is still running");
}
