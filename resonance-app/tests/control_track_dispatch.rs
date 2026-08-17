//! Every `track.*` / `mixer.*` method is still routed to a handler (ba
//! todo #1253).
//!
//! `update/control/track.rs` was split into per-concern submodules
//! (`lifecycle` / `output` / `sends` / `chain` / `params` / `mixer` /
//! `sidechain`) behind one dispatch table. Dropping an arm from that
//! table is silent: the request falls through every namespace, reaches
//! the `is_protocol_method` fallback and comes back as `unsupported`
//! ("method not implemented yet") — a wire answer that looks like a
//! deliberately unbuilt feature rather than a routing hole.
//!
//! So walk the protocol's own method lists and assert none of them lands
//! in that fallback. The params are deliberately empty: what is under
//! test is that *some* handler took the request (any validation error is
//! a pass), not what it did with it.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_control::methods::{mixer, track};
use resonance_control::{ErrorKind, Request, Response};

fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-track-dispatch.rprj"));
    app
}

fn call(app: &mut Resonance, method: &str) -> Response {
    let request = Request::new(1, method, &serde_json::json!({})).expect("params serialize");
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request,
        reply,
    })));
    rx.try_recv().expect("one reply per request")
}

/// Both wire answers that mean "nothing handled this method": the
/// `is_protocol_method` fallback ("not implemented yet") and the
/// end-of-dispatch `method_not_found` ("unknown method"). Both carry
/// kind `unsupported`, so the message is what separates them from a
/// handler that genuinely refuses the operation.
fn is_unrouted(response: &Response) -> bool {
    response.error.as_ref().is_some_and(|e| {
        e.kind() == ErrorKind::Unsupported
            && (e.message.contains("not implemented yet")
                || e.message.contains("unknown method"))
    })
}

#[test]
fn every_track_method_reaches_a_handler() {
    let mut app = app();
    for method in track::METHODS {
        let response = call(&mut app, method);
        assert!(
            !is_unrouted(&response),
            "{method} is not routed to a handler: {:?}",
            response.error
        );
    }
}

#[test]
fn every_mixer_method_reaches_a_handler() {
    let mut app = app();
    for method in mixer::METHODS {
        let response = call(&mut app, method);
        assert!(
            !is_unrouted(&response),
            "{method} is not routed to a handler: {:?}",
            response.error
        );
    }
}

/// The assertion above has teeth: a name in the same namespace that the
/// protocol does NOT define still comes back unrouted.
#[test]
fn an_undefined_track_method_is_reported_unrouted() {
    let mut app = app();
    let response = call(&mut app, "track.no_such_method");
    assert!(
        is_unrouted(&response),
        "an undefined method should not be handled: {response:?}"
    );
}
