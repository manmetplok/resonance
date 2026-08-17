//! Coverage for the versioned-backup helpers in `project.rs`:
//! `write_backup` rotation/pruning and `list_backups` newest-first
//! ordering. The clock is injected (callers pass the RFC3339 timestamp)
//! so these run deterministically without sleeping or mocking time.

use std::path::{Path, PathBuf};

use resonance_app::project::{list_backups, write_backup};

/// A unique scratch directory under the OS temp dir. Each call gets a
/// distinct name (process id + a monotonically increasing counter) so
/// concurrently-running tests never collide. Dropped at the end of the
/// test process; the OS reclaims temp space.
fn scratch_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "resonance-backup-test-{}-{tag}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Write a `project.json` into `dir` carrying `marker` so a restored
/// snapshot can be told apart from later saves.
fn write_project_json(dir: &Path, marker: &str) {
    std::fs::write(dir.join("project.json"), marker.as_bytes()).expect("write project.json");
}

#[test]
fn write_backup_snapshots_project_json() {
    let dir = scratch_dir("snapshot");
    write_project_json(&dir, "version-A");

    let dest = write_backup(&dir, "2026-01-01T00:00:00Z", 10).expect("write backup");

    assert!(dest.exists(), "backup file should exist");
    assert_eq!(
        dest,
        dir.join("backups").join("project-2026-01-01T00:00:00Z.json"),
        "backup uses the project-<timestamp>.json naming"
    );
    assert_eq!(
        std::fs::read_to_string(&dest).unwrap(),
        "version-A",
        "backup is a byte-for-byte snapshot of project.json"
    );
}

#[test]
fn list_backups_orders_newest_first() {
    let dir = scratch_dir("ordering");
    write_project_json(&dir, "data");

    // Deliberately write out of chronological order to prove the sort
    // is on the timestamp, not insertion or directory order.
    write_backup(&dir, "2026-01-02T00:00:00Z", 10).unwrap();
    write_backup(&dir, "2026-01-04T00:00:00Z", 10).unwrap();
    write_backup(&dir, "2026-01-01T00:00:00Z", 10).unwrap();
    write_backup(&dir, "2026-01-03T00:00:00Z", 10).unwrap();

    let stamps: Vec<String> = list_backups(&dir).into_iter().map(|e| e.timestamp).collect();
    assert_eq!(
        stamps,
        vec![
            "2026-01-04T00:00:00Z",
            "2026-01-03T00:00:00Z",
            "2026-01-02T00:00:00Z",
            "2026-01-01T00:00:00Z",
        ],
        "list_backups returns snapshots newest-first"
    );
}

#[test]
fn list_backups_sorts_chronologically_across_subsecond_precision() {
    let dir = scratch_dir("subsecond");
    write_project_json(&dir, "data");

    // A whole-second stamp is chronologically *before* the fractional
    // one in the same second, but sorts *after* it as plain text
    // ('Z' > '.'). Parsing the RFC3339 stamp must order them correctly.
    write_backup(&dir, "2026-01-01T00:00:00Z", 10).unwrap();
    write_backup(&dir, "2026-01-01T00:00:00.5Z", 10).unwrap();

    let stamps: Vec<String> = list_backups(&dir).into_iter().map(|e| e.timestamp).collect();
    assert_eq!(
        stamps,
        vec!["2026-01-01T00:00:00.5Z", "2026-01-01T00:00:00Z"],
        "the later fractional-second snapshot sorts first"
    );
}

#[test]
fn write_backup_prunes_to_retention_keeping_newest() {
    let dir = scratch_dir("prune");
    write_project_json(&dir, "data");

    // Six saves, retain three: only the three newest survive.
    for day in 1..=6 {
        write_backup(&dir, &format!("2026-01-0{day}T00:00:00Z"), 3).unwrap();
    }

    let stamps: Vec<String> = list_backups(&dir).into_iter().map(|e| e.timestamp).collect();
    assert_eq!(
        stamps,
        vec![
            "2026-01-06T00:00:00Z",
            "2026-01-05T00:00:00Z",
            "2026-01-04T00:00:00Z",
        ],
        "pruning keeps exactly the `retention` newest snapshots"
    );
}

#[test]
fn write_backup_prunes_out_of_order_writes() {
    let dir = scratch_dir("prune-unordered");
    write_project_json(&dir, "data");

    // Write a newer snapshot first, then an older one, with retention 1.
    // The older write must prune itself, leaving the genuinely-newest.
    write_backup(&dir, "2026-05-10T00:00:00Z", 1).unwrap();
    write_backup(&dir, "2026-05-01T00:00:00Z", 1).unwrap();

    let stamps: Vec<String> = list_backups(&dir).into_iter().map(|e| e.timestamp).collect();
    assert_eq!(
        stamps,
        vec!["2026-05-10T00:00:00Z"],
        "pruning is by timestamp, not write order"
    );
}

#[test]
fn list_backups_empty_when_no_backups_dir() {
    let dir = scratch_dir("empty");
    write_project_json(&dir, "data");

    assert!(
        list_backups(&dir).is_empty(),
        "a project with no backups/ dir lists nothing"
    );
}

#[test]
fn list_backups_ignores_unrelated_and_tmp_files() {
    let dir = scratch_dir("noise");
    write_project_json(&dir, "data");
    write_backup(&dir, "2026-01-01T00:00:00Z", 10).unwrap();

    // Drop noise into backups/: an interrupted atomic-write tmp and an
    // unrelated file. Neither should appear in the listing.
    let backups = dir.join("backups");
    std::fs::write(backups.join("project-2026-09-09T00:00:00Z.json.tmp"), b"x").unwrap();
    std::fs::write(backups.join("notes.txt"), b"x").unwrap();

    let stamps: Vec<String> = list_backups(&dir).into_iter().map(|e| e.timestamp).collect();
    assert_eq!(
        stamps,
        vec!["2026-01-01T00:00:00Z"],
        "only project-<timestamp>.json snapshots are listed"
    );
}
