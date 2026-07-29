//! Control-endpoint dispatch coverage (ba doc #265, todo #1147): the
//! `Message::Control` handler driven directly through the real
//! `update()` path with synthesized requests — no live socket needed.
//!
//! Covers the `control.hello` handshake (version check, capability
//! report, incompatible-connection rejection), the dispatch skeleton
//! (known-but-unimplemented vs unknown methods), connection bookkeeping,
//! and the monotonic revision counter bumped per undoable transaction.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::{ChordTrackMessage, Message};
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_control::methods::control::{HelloParams, HelloResult};
use resonance_control::rpc::codes;
use resonance_control::{ErrorKind, Request, Response, PROTOCOL_VERSION};

fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (app, _task) = Resonance::new();
    app
}

/// An app with an active project and a saved path, so edits are not
/// gated and undo recording is live.
fn app_with_project() -> Resonance {
    let mut app = app();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-endpoint-test.rprj"));
    app
}

/// Drive one request through the full `update()` path and return the
/// reply the handler sent.
fn roundtrip(app: &mut Resonance, conn: u64, request: Request) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn,
        request,
        reply,
    })));
    rx.try_recv().expect("every request gets exactly one reply")
}

fn hello_request(id: i64, protocol_version: u32) -> Request {
    Request::new(id, "control.hello", &HelloParams { protocol_version })
        .expect("hello params serialize")
}

// ---------------- control.hello ----------------

#[test]
fn hello_reports_version_and_capabilities() {
    // Deliberately no active project: the startup modal gate must never
    // swallow the control envelope.
    let mut app = app();
    let response = roundtrip(&mut app, 1, hello_request(1, PROTOCOL_VERSION));
    let result: HelloResult = response.result().expect("hello succeeds");
    assert_eq!(result.protocol_version, PROTOCOL_VERSION);
    assert!(!result.app_version.is_empty());
    assert!(result.capabilities.iter().any(|m| m == "control.hello"));
    assert!(result.capabilities.iter().any(|m| m == "song.summary"));
    assert!(result.capabilities.iter().any(|m| m == "transport.play"));
}

#[test]
fn hello_with_missing_params_is_invalid_params() {
    let mut app = app();
    let response = roundtrip(&mut app, 1, Request::without_params(1, "control.hello"));
    let error = response.error.expect("missing params rejected");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
}

#[test]
fn incompatible_version_rejects_connection() {
    let mut app = app();
    let _ = app.update(Message::Control(ControlMessage::Connected { conn: 7 }));

    let response = roundtrip(&mut app, 7, hello_request(1, PROTOCOL_VERSION + 1));
    let error = response.error.expect("version mismatch rejected");
    assert_eq!(error.kind(), ErrorKind::Unsupported);

    // Every subsequent request on that connection is rejected too.
    let response = roundtrip(&mut app, 7, Request::without_params(2, "song.summary"));
    let error = response.error.expect("incompatible connection rejected");
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.message.contains("incompatible"));

    // ...but the handshake itself stays answerable: a corrected hello
    // rehabilitates the connection (song.summary answers again, #1148).
    let response = roundtrip(&mut app, 7, hello_request(3, PROTOCOL_VERSION));
    assert!(response.result::<HelloResult>().is_ok());
    let response = roundtrip(&mut app, 7, Request::without_params(4, "song.summary"));
    assert!(response.result::<serde_json::Value>().is_ok());
}

// ---------------- dispatch skeleton ----------------

#[test]
fn every_advertised_capability_is_answered() {
    let mut app = app();
    // Every method in the hello capability list must produce a reply
    // that is either a result or a *stable* error kind — implemented
    // namespaces answer for real, pending ones reply with the
    // `unsupported` "not implemented yet" stub, and methods with
    // required params reject them precisely. `method_not_found` for an
    // advertised method would mean capabilities and dispatch drifted.
    for (i, method) in resonance_control::methods::capabilities().iter().enumerate() {
        let response = roundtrip(
            &mut app,
            1,
            Request::without_params(i as i64, *method),
        );
        let Some(error) = response.error else {
            continue; // implemented and succeeded
        };
        assert_ne!(
            error.code,
            codes::METHOD_NOT_FOUND,
            "{method} is advertised but unroutable"
        );
        assert!(
            matches!(
                error.kind(),
                ErrorKind::Unsupported
                    | ErrorKind::InvalidParams
                    | ErrorKind::NotFound
                    | ErrorKind::NeedsConfirmation
                    | ErrorKind::Busy
            ),
            "{method} replied with unstable error kind {:?}",
            error.kind()
        );
    }
}

#[test]
fn unknown_method_is_method_not_found() {
    let mut app = app();
    let response = roundtrip(&mut app, 1, Request::without_params(1, "nope.definitely_not"));
    let error = response.error.expect("unknown method rejected");
    assert_eq!(error.code, codes::METHOD_NOT_FOUND);
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    // The id is echoed even on errors.
    assert_eq!(response.id, Some(1i64.into()));
}

// ---------------- connection bookkeeping ----------------

#[test]
fn connect_disconnect_tracks_client_count() {
    let mut app = app();
    assert_eq!(app.control_client_count(), 0);
    let _ = app.update(Message::Control(ControlMessage::Connected { conn: 1 }));
    let _ = app.update(Message::Control(ControlMessage::Connected { conn: 2 }));
    assert_eq!(app.control_client_count(), 2);
    let _ = app.update(Message::Control(ControlMessage::Disconnected { conn: 1 }));
    assert_eq!(app.control_client_count(), 1);
    let _ = app.update(Message::Control(ControlMessage::Disconnected { conn: 2 }));
    assert_eq!(app.control_client_count(), 0);
}

// ---------------- revision counter ----------------

#[test]
fn revision_bumps_once_per_undoable_transaction() {
    let mut app = app_with_project();
    assert_eq!(app.revision(), 0);

    // An undoable edit through the normal update path bumps once.
    let _ = app.update(Message::ChordTrack(ChordTrackMessage::AddRegion {
        start_sample: 0,
        end_sample: 44_100,
        symbol: "C".to_owned(),
    }));
    assert_eq!(app.revision(), 1);

    // Non-mutating traffic doesn't bump: a control read and a hello.
    let _ = roundtrip(&mut app, 1, hello_request(1, PROTOCOL_VERSION));
    let _ = roundtrip(&mut app, 1, Request::without_params(2, "song.summary"));
    assert_eq!(app.revision(), 1);

    // Undo restores the previous state — that's a committed change for
    // remote revision tracking.
    let _ = app.update(Message::Undo);
    assert_eq!(app.revision(), 2);
}
