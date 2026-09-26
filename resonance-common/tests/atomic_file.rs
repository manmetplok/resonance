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
    assert_eq!(tmp_files(dir.path()), Vec::<String>::new());
}

/// Every `*.tmp` file in `dir`, by name.
fn tmp_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect()
}

/// LIB-08: two writers of the same target must not share a temp file.
/// With a fixed `<name>.tmp`, one writer's `create` truncates the other's
/// in-progress temp, and the renames race — publishing a spliced file or
/// failing with ENOENT. Every write must succeed, and every read of the
/// target must be exactly one of the two payloads.
#[test]
fn concurrent_writers_never_publish_a_spliced_file() {
    let dir = TempDir::new("race");
    let target = dir.path().join("settings.json");
    let a = vec![b'a'; 64 * 1024];
    let b = vec![b'b'; 48 * 1024];
    atomic_write(&target, &a).expect("seed write");

    std::thread::scope(|s| {
        for payload in [&a, &b] {
            let target = &target;
            s.spawn(move || {
                for i in 0..300 {
                    atomic_write(target, payload)
                        .unwrap_or_else(|e| panic!("write {i} failed: {e}"));
                }
            });
        }
        let target = &target;
        let (a, b) = (&a, &b);
        s.spawn(move || {
            for _ in 0..600 {
                let got = std::fs::read(target).expect("target always exists");
                assert!(
                    got == *a || got == *b,
                    "published a spliced/truncated file ({} bytes)",
                    got.len()
                );
            }
        });
    });

    let got = std::fs::read(&target).unwrap();
    assert!(got == a || got == b);
    assert_eq!(tmp_files(dir.path()), Vec::<String>::new(), "temp files leaked");
}

const CHILD_ENV: &str = "RESONANCE_ATOMIC_WRITE_FSIZE_CHILD";

/// LIB-08: a write that fails part-way (here: `RLIMIT_FSIZE`, standing in
/// for a full disk / EDQUOT) must return an error, leave the existing
/// target untouched and remove its partial temp file. Runs the body in a
/// child process so the file-size limit can't affect anything else; the
/// child ignores SIGXFSZ (an ignored disposition survives `exec`), so the
/// oversized write fails with EFBIG instead of killing it.
#[test]
fn failed_write_removes_its_temp_file() {
    let dir = TempDir::new("fsize");
    let exe = std::env::current_exe().unwrap();
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg("trap '' XFSZ; ulimit -f 16; exec \"$0\" --exact failed_write_child --nocapture --test-threads=1")
        .arg(&exe)
        .env(CHILD_ENV, dir.path())
        .status()
        .expect("spawn child");
    assert!(status.success(), "child assertions failed: {status}");
}

/// Child half of [`failed_write_removes_its_temp_file`]; a no-op unless
/// launched by it.
#[test]
fn failed_write_child() {
    let Some(dir) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let dir = PathBuf::from(dir);
    let target = dir.join("project.json");
    atomic_write(&target, b"the good content").expect("small write fits the limit");

    // Far over the 16-block limit: `write_all` fails part-way through.
    let big = vec![b'x'; 4 * 1024 * 1024];
    let result = atomic_write(&target, &big);
    assert!(result.is_err(), "an oversized write must fail under RLIMIT_FSIZE");
    assert_eq!(std::fs::read(&target).unwrap(), b"the good content");
    assert_eq!(tmp_files(&dir), Vec::<String>::new(), "partial temp file left behind");
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
