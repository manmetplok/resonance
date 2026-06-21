//! Coverage for crash-safe project writes
//! (`project::atomic_write` and `project::save_project`).
//!
//! Todo #461 (doc #171 "Atomic writes"): every JSON/blob write in the
//! save path must be write-temp-then-rename so a crash mid-save leaves
//! either the prior or the new file fully intact — never a truncated
//! `project.json`. These tests pin three properties:
//!
//!   1. `atomic_write` round-trips: bytes written are bytes read back,
//!      whether the target is new or overwritten in place.
//!   2. A successful write leaves no `*.tmp` litter behind.
//!   3. A leftover `*.tmp` from an interrupted write is inert — it
//!      never clobbers the good file the loader actually reads, and a
//!      full `save_project` -> `load_project` round-trip survives it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use resonance_app::project::{self, ProjectFile};

/// A unique temp directory that deletes itself when dropped. Avoids a
/// `tempfile` dependency (not present in this crate) while keeping each
/// test isolated and leaving no litter behind on success or panic.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "resonance_atomic_{tag}_{}_{n}",
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

#[test]
fn atomic_write_round_trips_new_file() {
    let dir = TempDir::new("new");
    let target = dir.path().join("hello.bin");

    project::atomic_write(&target, b"first contents").expect("atomic write");

    let read = std::fs::read(&target).expect("read back");
    assert_eq!(read, b"first contents");
}

#[test]
fn atomic_write_overwrites_in_place_and_leaves_no_tmp() {
    let dir = TempDir::new("overwrite");
    let target = dir.path().join("project.json");

    project::atomic_write(&target, b"old payload").expect("first write");
    project::atomic_write(&target, b"new payload that is longer").expect("second write");

    let read = std::fs::read(&target).expect("read back");
    assert_eq!(read, b"new payload that is longer");

    // A clean save must not strand a temp file in the directory.
    let tmp = dir.path().join("project.json.tmp");
    assert!(
        !tmp.exists(),
        "successful atomic_write should leave no .tmp behind"
    );
}

#[test]
fn leftover_tmp_does_not_clobber_good_file() {
    let dir = TempDir::new("staletmp");
    let target = dir.path().join("project.json");

    // The good, fully-written file.
    project::atomic_write(&target, b"the good file").expect("write good file");

    // Simulate a crash mid-save: a truncated/garbage sibling .tmp that
    // never got renamed over the target.
    std::fs::write(dir.path().join("project.json.tmp"), b"garbage half-write")
        .expect("write stale tmp");

    // The loader keys off the fixed `project.json` name, so the stale
    // .tmp is inert: the good contents are still what we read.
    let read = std::fs::read(&target).expect("read back");
    assert_eq!(read, b"the good file");

    // A subsequent atomic write still succeeds over the good file even
    // with the stale tmp present, and reuses (overwrites) that tmp name.
    project::atomic_write(&target, b"the better file").expect("rewrite over stale tmp");
    let read = std::fs::read(&target).expect("read back 2");
    assert_eq!(read, b"the better file");
    assert!(
        !dir.path().join("project.json.tmp").exists(),
        "atomic_write should consume/rename its tmp, leaving none"
    );
}

#[test]
fn save_project_round_trips_through_load() {
    let dir = TempDir::new("save");
    let project_dir = dir.path().join("MyProject.rproj");
    std::fs::create_dir_all(&project_dir).expect("create project dir");

    let mut file = ProjectFile {
        bpm: 132.0,
        time_sig_num: 3,
        time_sig_den: 4,
        ..ProjectFile::default()
    };
    file.master_volume = -6.0;

    project::save_project(&project_dir, &file, &[], &[]).expect("save");

    // project.json exists and no .tmp litter remains beside it.
    assert!(project_dir.join("project.json").exists());
    assert!(!project_dir.join("project.json.tmp").exists());

    let loaded = project::load_project(&project_dir).expect("load");
    assert_eq!(loaded.file.bpm, 132.0);
    assert_eq!(loaded.file.time_sig_num, 3);
    assert_eq!(loaded.file.time_sig_den, 4);
    assert_eq!(loaded.file.master_volume, -6.0);
    assert_eq!(loaded.file.version, project::PROJECT_FORMAT_VERSION);
}

#[test]
fn save_project_survives_a_leftover_tmp_from_a_prior_crash() {
    let dir = TempDir::new("savecrash");
    let project_dir = dir.path().join("MyProject.rproj");
    std::fs::create_dir_all(&project_dir).expect("create project dir");

    let first = ProjectFile {
        bpm: 100.0,
        ..ProjectFile::default()
    };
    project::save_project(&project_dir, &first, &[], &[]).expect("first save");

    // Drop a stale tmp next to the good file, as an interrupted save
    // would have.
    std::fs::write(project_dir.join("project.json.tmp"), b"{ truncated")
        .expect("write stale tmp");

    // The good prior file still loads despite the stale tmp.
    let recovered = project::load_project(&project_dir).expect("load after crash");
    assert_eq!(recovered.file.bpm, 100.0);

    // A new save cleanly replaces it and clears the tmp.
    let second = ProjectFile {
        bpm: 140.0,
        ..ProjectFile::default()
    };
    project::save_project(&project_dir, &second, &[], &[]).expect("second save");
    assert!(!project_dir.join("project.json.tmp").exists());

    let loaded = project::load_project(&project_dir).expect("load");
    assert_eq!(loaded.file.bpm, 140.0);
}
