//! Real unix-socket round trip for the control endpoint (ba doc #265,
//! todo #1147): a raw `UnixStream` client says hello and calls a stub
//! method against the live listener, with the test pumping the bridge
//! channel into `update()` exactly like the iced subscription does.
//!
//! Also covers the lifecycle contract: stale-socket replacement on bind
//! and socket-file removal on clean shutdown (server drop).

use iced::futures::channel::mpsc::UnboundedReceiver;
use resonance_app::control_socket::{self, ControlMessage};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_control::methods::control::{HelloParams, HelloResult};
use resonance_control::{ErrorKind, MessageReader, Request, Response, PROTOCOL_VERSION};
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::{Duration, Instant};

/// A temp dir the server will accept as a socket dir: private (0700).
/// `tempfile` creates its dirs with the umask's default mode, and the
/// server refuses an open socket dir rather than chmodding it (CTL-11).
pub(crate) fn private_tempdir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
        .expect("make temp dir private");
    dir
}

/// Wait (bounded) for the next socket-thread event on the bridge.
fn next_event(rx: &mut UnboundedReceiver<ControlMessage>) -> ControlMessage {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match rx.try_recv() {
            Ok(message) => return message,
            // Empty (or closed — which would spin into the timeout below,
            // still a clean failure): wait and retry.
            Err(_) => {
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for a bridge event"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

#[test]
fn socket_hello_and_stub_method_round_trip() {
    let dir = private_tempdir();
    let path = dir.path().join("control.sock");

    let (tx, mut rx) = iced::futures::channel::mpsc::unbounded();
    let jobs = std::sync::Arc::new(resonance_app::control_jobs::JobBoard::default());
    let server = control_socket::spawn(path.clone(), tx, jobs).expect("bind control socket");
    assert!(path.exists(), "socket file exists while serving");

    // Raw client: hello, then a known-but-unimplemented method, then an
    // unknown one — all written before the server replies, so the reply
    // order proves requests are serialized in arrival order.
    let client = UnixStream::connect(&path).expect("connect");
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout");
    let mut writer = client.try_clone().expect("clone stream");
    let hello = Request::new(
        1,
        "control.hello",
        &HelloParams {
            protocol_version: PROTOCOL_VERSION,
        },
    )
    .expect("hello request");
    resonance_control::write_message(&mut writer, &hello).expect("send hello");
    resonance_control::write_message(&mut writer, &Request::without_params(2, "song.summary"))
        .expect("send stub call");
    resonance_control::write_message(&mut writer, &Request::without_params(3, "nope.method"))
        .expect("send unknown call");

    // The test is the update loop: pump each bridge event through the
    // real `update()` path, exactly like the subscription does.
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let connected = next_event(&mut rx);
    assert!(matches!(connected, ControlMessage::Connected { .. }));
    let _ = app.update(Message::Control(connected));
    assert_eq!(app.control_client_count(), 1);
    for _ in 0..3 {
        let event = next_event(&mut rx);
        assert!(matches!(event, ControlMessage::Request(_)));
        let _ = app.update(Message::Control(event));
    }

    // Client side: three replies, in request order, ids echoed.
    let mut reader = MessageReader::from_reader(client);
    let first: Response = reader
        .read_message()
        .expect("read hello reply")
        .expect("hello reply present");
    assert_eq!(first.id, Some(1i64.into()));
    let hello_result: HelloResult = first.result().expect("hello succeeds");
    assert_eq!(hello_result.protocol_version, PROTOCOL_VERSION);
    assert!(!hello_result.capabilities.is_empty());

    let second: Response = reader
        .read_message()
        .expect("read stub reply")
        .expect("stub reply present");
    assert_eq!(second.id, Some(2i64.into()));
    // song.summary is a real view since todo #1148 — a raw socket
    // client gets the introspection JSON back.
    let summary: serde_json::Value = second.result().expect("song.summary succeeds");
    assert!(summary.get("revision").is_some());

    let third: Response = reader
        .read_message()
        .expect("read unknown-method reply")
        .expect("unknown-method reply present");
    assert_eq!(third.id, Some(3i64.into()));
    assert_eq!(
        third.error.expect("unknown method rejected").kind(),
        ErrorKind::Unsupported
    );

    // Clean shutdown removes the socket file.
    drop(reader);
    drop(writer);
    drop(server);
    assert!(!path.exists(), "socket file removed on clean shutdown");
}

#[test]
fn stale_socket_is_replaced_on_bind() {
    let dir = private_tempdir();
    let path = dir.path().join("control.sock");

    // Simulate a crashed instance: a socket file nobody accepts on.
    let stale = UnixListener::bind(&path).expect("bind stale");
    drop(stale); // file stays behind, no accepting listener
    assert!(path.exists(), "stale socket file left on disk");

    let (tx, mut rx) = iced::futures::channel::mpsc::unbounded();
    let jobs = std::sync::Arc::new(resonance_app::control_jobs::JobBoard::default());
    let server = control_socket::spawn(path.clone(), tx, jobs).expect("stale socket replaced");

    // The rebound listener actually accepts.
    let _client = UnixStream::connect(&path).expect("connect to rebound socket");
    assert!(matches!(
        next_event(&mut rx),
        ControlMessage::Connected { .. }
    ));

    drop(server);
    assert!(!path.exists());
}

/// Spawn a server on `path` with a throwaway bridge/job board, keeping
/// the bridge receiver alive (dropping it would look like app
/// shutdown to the accept loop).
fn spawn_server(
    path: &std::path::Path,
) -> (
    io::Result<control_socket::ControlServer>,
    UnboundedReceiver<ControlMessage>,
) {
    let (tx, rx) = iced::futures::channel::mpsc::unbounded();
    let jobs = std::sync::Arc::new(resonance_app::control_jobs::JobBoard::default());
    (control_socket::spawn(path.to_path_buf(), tx, jobs), rx)
}

#[test]
fn second_instance_is_refused_while_lock_held() {
    let dir = private_tempdir();
    let path = dir.path().join("control.sock");

    let (winner, _winner_rx) = spawn_server(&path);
    let winner = winner.expect("first instance binds");

    // A concurrent second instance loses the instance lock and is
    // refused up front — its unlink can never reach the winner's
    // freshly bound socket.
    let (loser, _loser_rx) = spawn_server(&path);
    let err = loser.expect_err("second instance refused while first serves");
    assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
    assert!(
        err.to_string().contains("another resonance instance"),
        "loser reports the live instance: {err}"
    );

    // The winner's socket survived the loser's failed start.
    assert!(path.exists(), "winner's socket file still published");
    let _client = UnixStream::connect(&path).expect("winner still accepts");

    // Clean shutdown releases the lock: a successor binds fine.
    drop(winner);
    assert!(!path.exists(), "socket removed on clean shutdown");
    let (successor, _successor_rx) = spawn_server(&path);
    let successor = successor.expect("lock released on drop, successor binds");
    drop(successor);
    assert!(!path.exists());
}

#[test]
fn superseded_drop_leaves_successor_socket_alone() {
    let dir = private_tempdir();
    let path = dir.path().join("control.sock");

    let (first, _first_rx) = spawn_server(&path);
    let first = first.expect("first instance binds");

    // Simulate the first instance being superseded: an external
    // cleanup (tmpfiles-style) purges both the socket and its lock
    // file, so a second instance takes a fresh lock and rebinds the
    // path while the first is still alive.
    std::fs::remove_file(&path).expect("purge socket file");
    std::fs::remove_file(dir.path().join("control.sock.lock")).expect("purge lock file");
    let (second, mut second_rx) = spawn_server(&path);
    let second = second.expect("second instance rebinds after purge");
    assert!(path.exists(), "second instance's socket published");

    // Dropping the superseded instance must NOT unpublish the live
    // server: the path no longer points at the file it bound.
    drop(first);
    assert!(
        path.exists(),
        "superseded drop left the successor's socket in place"
    );
    let _client = UnixStream::connect(&path).expect("successor still accepts");
    assert!(matches!(
        next_event(&mut second_rx),
        ControlMessage::Connected { .. }
    ));

    // The live owner's own drop still cleans up.
    drop(second);
    assert!(!path.exists(), "owner's drop removes its socket");
}

// ---- socket directory trust (code review CTL-11 / UPD-12) -----------------

fn dir_mode(p: &std::path::Path) -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).unwrap().mode() & 0o7777
}

/// `/tmp/resonance-<uid>` pre-planted as a symlink to a directory the
/// user owns: the server must refuse, not chmod the target and bind in it.
#[test]
fn a_symlinked_socket_dir_is_refused_and_its_target_left_alone() {
    use std::os::unix::fs::PermissionsExt;
    let root = private_tempdir();
    let target = root.path().join("project");
    std::fs::create_dir(&target).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    let link = root.path().join("resonance-1000");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let (server, _rx) = spawn_server(&link.join("control.sock"));
    let err = server.err().expect("spawn must refuse a symlinked socket dir");
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{err}");
    assert_eq!(dir_mode(&target), 0o755, "symlink target must not be chmodded");
    assert!(!target.join("control.sock").exists(), "nothing bound in the target");
}

/// An existing directory with group/other access (a pre-created
/// `/tmp/resonance-<uid>`, or an override pointing into a project dir)
/// is refused rather than silently tightened.
#[test]
fn a_pre_existing_open_socket_dir_is_refused_not_chmodded() {
    use std::os::unix::fs::PermissionsExt;
    let root = private_tempdir();
    for mode in [0o755, 0o777] {
        let dir = root.path().join(format!("d{mode:o}"));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).unwrap();
        let (server, _rx) = spawn_server(&dir.join("control.sock"));
        let err = server.err().expect("open socket dir refused");
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{err}");
        assert_eq!(dir_mode(&dir), mode, "must not be chmodded");
    }
}

/// A missing socket dir is created private and served from.
#[test]
fn a_missing_socket_dir_is_created_0700() {
    let root = private_tempdir();
    let dir = root.path().join("run/resonance");
    let (server, _rx) = spawn_server(&dir.join("control.sock"));
    let _server = server.expect("spawn creates its dir");
    assert_eq!(dir_mode(&dir), 0o700);
}
