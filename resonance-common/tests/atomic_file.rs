//! Coverage for the shared crash-safe write primitive
//! (`resonance_common::atomic_file`), lifted out of
//! `resonance-app::project::io::atomic_write` so every user-state store
//! (installed-content registry, controller-map presets, app settings,
//! recent-projects list, track presets) gets the same write-temp-then-
//! rename guarantee a project save already had.
//!
//! What a crash mid-write actually leaves on disk isn't directly
//! testable (that needs killing the process between the `write` and the
//! `rename`); instead these pin the properties that make the guarantee
//! hold: a successful write fully replaces the old content, and a
//! failed write (no permission on the target directory) never touches
//! whatever was already there.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use resonance_common::atomic_write;

/// A unique temp directory that deletes itself when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "resonance_common_atomic_{tag}_{}_{n}",
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
        // The read-only-dir test below strips write permission; restore
        // it so removal doesn't fail on the way out.
        let mut perms = match std::fs::metadata(&self.0) {
            Ok(m) => m.permissions(),
            Err(_) => return,
        };
        perms.set_mode(0o755);
        let _ = std::fs::set_permissions(&self.0, perms);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn successful_write_replaces_existing_content() {
    let dir = TempDir::new("replace");
    let target = dir.path().join("state.json");

    atomic_write(&target, b"old").expect("first write");
    assert_eq!(std::fs::read(&target).unwrap(), b"old");

    atomic_write(&target, b"new content, longer than the old").expect("second write");
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"new content, longer than the old"
    );

    // No stray tmp left behind.
    assert!(!dir.path().join("state.json.tmp").exists());
}

#[test]
fn write_into_read_only_dir_errors_without_clobbering_existing_file() {
    let dir = TempDir::new("readonly");
    let target = dir.path().join("state.json");

    // A good file already on disk.
    atomic_write(&target, b"the good content").expect("write while writable");

    // Lock the directory: the temp file's `create` can no longer
    // succeed in it.
    let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
    perms.set_mode(0o555);
    std::fs::set_permissions(dir.path(), perms).expect("chmod dir read-only");

    let result = atomic_write(&target, b"this must never land");

    // Restore write permission before any more filesystem calls,
    // including the ones this assertion below makes.
    let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(dir.path(), perms).expect("chmod dir back to writable");

    assert!(
        result.is_err(),
        "a write that can't create its tmp file must fail, not silently drop data"
    );
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"the good content",
        "the existing file must survive a failed write untouched"
    );
}
