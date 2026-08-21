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
