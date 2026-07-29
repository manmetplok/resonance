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
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_control::methods::control::{HelloParams, HelloResult};
use resonance_control::{ErrorKind, MessageReader, Request, Response, PROTOCOL_VERSION};
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::{Duration, Instant};

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
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("control.sock");

    let (tx, mut rx) = iced::futures::channel::mpsc::unbounded();
    let server = control_socket::spawn(path.clone(), tx).expect("bind control socket");
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
    let (mut app, _task) = Resonance::new();
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
    let error = second.error.expect("stub is unsupported");
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.message.contains("not implemented yet"));

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
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("control.sock");

    // Simulate a crashed instance: a socket file nobody accepts on.
    let stale = UnixListener::bind(&path).expect("bind stale");
    drop(stale); // file stays behind, no accepting listener
    assert!(path.exists(), "stale socket file left on disk");

    let (tx, mut rx) = iced::futures::channel::mpsc::unbounded();
    let server = control_socket::spawn(path.clone(), tx).expect("stale socket replaced");

    // The rebound listener actually accepts.
    let _client = UnixStream::connect(&path).expect("connect to rebound socket");
    assert!(matches!(
        next_event(&mut rx),
        ControlMessage::Connected { .. }
    ));

    drop(server);
    assert!(!path.exists());
}
