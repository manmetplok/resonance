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

fn dispatch(app: &mut Resonance, m: ProjectIoMessage) {
    let _ = app.update(Message::ProjectIo(m));
}

#[test]
fn autosave_completion_keeps_dirty_and_records_autosave_time() {
    let (mut app, _task) = Resonance::new();

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
    let (mut app, _task) = Resonance::new();
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
    let (mut app, _task) = Resonance::new();
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
    let (mut app, _task) = Resonance::new();
    assert!(app.last_autosave_at().is_none());

    // No project path set → the autosave snapshots into a per-session
    // scratch dir under the cache directory. The write itself runs async
    // on the engine path, but the directory is created synchronously when
    // the save kicks off.
    dispatch(&mut app, ProjectIoMessage::Autosave);
    assert!(
        app.is_saving(),
        "autosave kicked off even with no project path"
    );

    let scratch = dirs::cache_dir()
        .expect("cache dir")
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
