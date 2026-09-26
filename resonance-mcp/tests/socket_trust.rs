//! The MCP client only talks to a control socket in a directory it can
//! trust (code review CTL-11): without `XDG_RUNTIME_DIR` the socket lives
//! in `/tmp/resonance-<uid>`, which another local user can pre-create —
//! and then listen on `control.sock` in it, feeding the model attacker-
//! chosen "tool results". A socket dir that is a symlink, someone else's,
//! or open to group/other is refused before anything is connected or sent.

#[allow(dead_code)]
mod common;

use common::FakeApp;
use resonance_control::PROTOCOL_VERSION;
use resonance_mcp::{CallError, ControlClient};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn scratch(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "resonance-mcp-trust-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    root
}

/// A listener at `dir/control.sock` counting accepted connections.
fn listen_in(dir: &Path) -> (PathBuf, Arc<AtomicU64>) {
    let path = dir.join("control.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let accepted = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&accepted);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if stream.is_ok() {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    (path, accepted)
}

fn assert_refused(path: PathBuf, accepted: &AtomicU64) {
    let client = ControlClient::new(path);
    let err = client.hello_blocking().expect_err("untrusted socket dir must be refused");
    assert!(
        matches!(err, CallError::UntrustedSocket { .. }),
        "expected UntrustedSocket, got {err:?}"
    );
    assert!(err.actionable_message().contains("0700"), "{}", err.actionable_message());
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(accepted.load(Ordering::SeqCst), 0, "must not even connect");
}

#[test]
fn a_socket_in_an_open_dir_is_refused() {
    let root = scratch("open");
    let dir = root.join("resonance-1000");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    let (path, accepted) = listen_in(&dir);
    assert_refused(path, &accepted);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_socket_behind_a_symlinked_dir_is_refused() {
    let root = scratch("link");
    let target = root.join("elsewhere");
    std::fs::create_dir(&target).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
    let link = root.join("resonance-1000");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let (_real, accepted) = listen_in(&target);
    assert_refused(link.join("control.sock"), &accepted);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_missing_socket_dir_still_reads_as_not_running() {
    let root = scratch("missing");
    let client = ControlClient::new(root.join("nope/control.sock"));
    let err = client.hello_blocking().expect_err("nothing there");
    assert!(matches!(err, CallError::NotRunning { .. }), "{err:?}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_socket_in_a_private_dir_is_used() {
    let app = FakeApp::spawn(PROTOCOL_VERSION, |_| None);
    let client = ControlClient::new(app.path());
    client.hello_blocking().expect("private socket dir is trusted");
}
