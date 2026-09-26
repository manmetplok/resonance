//! Control-socket path resolution (doc #265). The rule is shared by the
//! app and resonance-mcp precisely so they can't drift apart; these
//! tests pin the rule itself via the pure resolver.

use std::path::PathBuf;

use resonance_control::socket::resolve_socket_path;

#[test]
fn override_wins_verbatim() {
    let path = resolve_socket_path(
        Some("/custom/ctl.sock".into()),
        Some(PathBuf::from("/run/user/1000")),
        Some(PathBuf::from("/var/folders/xx/T")),
        1000,
    );
    assert_eq!(path, PathBuf::from("/custom/ctl.sock"));
}

#[test]
fn empty_override_is_ignored() {
    let path = resolve_socket_path(Some(String::new()), None, None, 1000);
    assert_eq!(path, PathBuf::from("/tmp/resonance-1000/control.sock"));
}

#[test]
fn runtime_dir_beats_tmpdir() {
    let path = resolve_socket_path(
        None,
        Some(PathBuf::from("/run/user/1000")),
        Some(PathBuf::from("/var/folders/xx/T")),
        1000,
    );
    assert_eq!(
        path,
        PathBuf::from("/run/user/1000/resonance/control.sock")
    );
}

#[test]
fn uid_fallback_uses_the_given_uid() {
    // The uid must be the caller's real uid, not the old /proc/self
    // read that yielded 0 on macOS and merged every user's socket dir.
    let path = resolve_socket_path(None, None, None, 502);
    assert_eq!(path, PathBuf::from("/tmp/resonance-502/control.sock"));
}

/// `$TMPDIR` is per-user on macOS and stands in for the never-set
/// `XDG_RUNTIME_DIR` there; other platforms skip straight to the uid
/// fallback.
#[test]
fn tmpdir_is_macos_only() {
    let path = resolve_socket_path(None, None, Some(PathBuf::from("/var/folders/xx/T")), 502);
    if cfg!(target_os = "macos") {
        assert_eq!(path, PathBuf::from("/var/folders/xx/T/resonance/control.sock"));
    } else {
        assert_eq!(path, PathBuf::from("/tmp/resonance-502/control.sock"));
    }
}

// ---- socket directory trust (code review CTL-11 / UPD-12) -----------------

#[cfg(unix)]
mod socket_dir {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    use resonance_control::socket::{prepare_socket_dir, verify_socket_dir};

    /// A fresh private scratch root per test (itself 0700).
    fn scratch(tag: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "resonance-ctl11-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        root
    }

    fn mode(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().mode() & 0o7777
    }

    #[test]
    fn a_missing_dir_is_created_private() {
        let root = scratch("create");
        let dir = root.join("nested/resonance");
        prepare_socket_dir(&dir).expect("created");
        assert_eq!(mode(&dir), 0o700);
        prepare_socket_dir(&dir).expect("an existing private dir is accepted");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_symlinked_dir_is_refused_and_its_target_left_alone() {
        let root = scratch("symlink");
        let target = root.join("victim");
        std::fs::create_dir(&target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        let link = root.join("resonance-1000");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let err = prepare_socket_dir(&link).expect_err("symlinked socket dir must be refused");
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{err}");
        assert!(verify_socket_dir(&link).is_err());
        assert_eq!(mode(&target), 0o755, "the symlink target must not be chmodded");

        // Even a private target is refused through the link.
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(prepare_socket_dir(&link).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_pre_created_dir_with_open_permissions_is_refused_not_chmodded() {
        let root = scratch("mode");
        for m in [0o755, 0o777, 0o710, 0o701] {
            let dir = root.join(format!("d{m:o}"));
            std::fs::create_dir(&dir).unwrap();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(m)).unwrap();
            let err = prepare_socket_dir(&dir).expect_err("open dir refused");
            assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
            assert!(verify_socket_dir(&dir).is_err());
            assert_eq!(mode(&dir), m, "must not be chmodded");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_dir_owned_by_someone_else_is_refused() {
        // `/` is root's; meaningless when the suite itself runs as root.
        // SAFETY: geteuid takes no arguments and cannot fail.
        if std::fs::metadata("/").unwrap().uid() == unsafe { libc::geteuid() } {
            return;
        }
        let err = verify_socket_dir(Path::new("/")).expect_err("foreign dir refused");
        assert!(err.to_string().contains("owned by uid"), "{err}");
    }

    #[test]
    fn a_non_directory_is_refused() {
        let root = scratch("file");
        let file = root.join("plain");
        std::fs::write(&file, b"").unwrap();
        assert!(verify_socket_dir(&file).is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
