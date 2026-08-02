//! Control-endpoint mutation gate coverage (ba doc #265, todo #1161).
//!
//! With no active project the app sits on the startup project-picker
//! modal, where a synthesized mutating domain message is silently
//! swallowed by the pre-dispatch gate. A remote client must instead see
//! a stable `busy` error — never a no-op that claims success (the
//! original bug: `transport.*` / `track.*` / `notes.*` / `render.*`
//! accepted the request, returned a success payload, and did nothing;
//! `track.add` even handed back a fake `track_id`).
//!
//! This locks the gate down structurally: EVERY method in a mutating
//! namespace's `METHODS` list must be rejected `busy` when
//! `has_active_project` is false, so a newly added mutating handler
//! cannot forget the gate. Read-only introspection (`song.*`, `job.*`,
//! `control.hello`) and the project-lifecycle methods (`project.*`,
//! which open/create the project the gate checks for) are exempt.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_control::methods;
use resonance_control::{ErrorKind, Request, Response};

/// A fresh app with NO active project (boot state: the startup modal).
fn app_no_project() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    // `Resonance::new()` boots onto the startup project-picker modal, so
    // `has_active_project` is false — exactly the gated state under test.
    let (app, _task) = Resonance::new();
    app
}

/// Drive one request through the full `update()` path and return the
/// single reply the handler sent.
fn roundtrip(app: &mut Resonance, request: Request) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request,
        reply,
    })));
    rx.try_recv().expect("every request gets exactly one reply")
}

/// The mutating namespaces: every method here needs an active project.
/// Sourced from `methods::*::METHODS` so a new method in any of them is
/// automatically covered.
fn mutating_methods() -> Vec<&'static str> {
    let mut methods = Vec::new();
    for namespace in [
        methods::transport::METHODS,
        methods::track::METHODS,
        methods::mixer::METHODS,
        // `master.summary` reads only, but it describes the OPEN
        // project's master bus, so like every mutating method it must
        // answer `busy` rather than report a default master for a
        // project that isn't there (ba doc #273).
        methods::master::METHODS,
        methods::bus::METHODS,
        // `edit.status` reads only, but like `master.summary` it
        // describes the OPEN project's history (ba doc #273).
        methods::edit::METHODS,
        methods::section::METHODS,
        methods::harmony::METHODS,
        methods::generate::METHODS,
        methods::notes::METHODS,
        methods::vocal::METHODS,
        methods::render::METHODS,
    ] {
        methods.extend_from_slice(namespace);
    }
    // `track.plugins` and `track.plugin_params` live in `track::METHODS`
    // but are read-only queries served by the `song::try_handle` block
    // above the gate — neither is a mutation. Everything else in
    // `track::*` is.
    methods.retain(|m| *m != methods::track::PLUGINS && *m != methods::track::PLUGIN_PARAMS);
    methods
}

/// The read-only / lifecycle allowlist: methods that legitimately run
/// with no active project and so must NOT be `busy`-gated.
fn allowlisted_methods() -> Vec<&'static str> {
    // `track.plugins` and `track.plugin_params` are the read-only
    // members of the otherwise-mutating `track::*` namespace.
    let mut methods = vec![
        "control.hello",
        methods::track::PLUGINS,
        methods::track::PLUGIN_PARAMS,
    ];
    for namespace in [
        methods::song::METHODS,
        methods::project::METHODS,
        resonance_control::job::METHODS,
    ] {
        methods.extend_from_slice(namespace);
    }
    methods
}

#[test]
fn every_mutating_method_is_busy_without_an_active_project() {
    let mutating = mutating_methods();
    assert!(!mutating.is_empty(), "expected mutating namespaces to be non-empty");

    for method in mutating {
        let mut app = app_no_project();
        // Empty params: the gate must fire BEFORE param validation, so a
        // client with no project always learns "no project" rather than a
        // misleading `invalid_params`.
        let response = roundtrip(&mut app, Request::without_params(1, method));
        let error = response
            .error
            .unwrap_or_else(|| panic!("{method} returned success with no active project"));
        assert_eq!(
            error.kind(),
            ErrorKind::Busy,
            "{method} must be `busy` with no active project, got {error:?}"
        );
    }
}

#[test]
fn every_mutating_method_is_covered_by_a_namespace_list() {
    // Guards against a mutating namespace being added to `capabilities()`
    // (so it ships in `control.hello`) but forgotten in this test's
    // `mutating_methods()`/`allowlisted_methods()` split — which would
    // otherwise let a new gate-less namespace slip through unchecked.
    let mut classified: Vec<&str> = mutating_methods();
    classified.extend(allowlisted_methods());
    for method in methods::capabilities() {
        assert!(
            classified.contains(&method),
            "{method} is in control.hello capabilities but neither mutating nor allowlisted; \
             add it to the gate test"
        );
    }
}

#[test]
fn allowlisted_methods_are_not_busy_gated() {
    // The allowlisted methods must not be rejected by the *mutation
    // gate*: with no project they may still fail for their own reasons
    // (bad params, missing file, ...), but never with a `busy` that comes
    // from `has_active_project == false`.
    for method in allowlisted_methods() {
        let mut app = app_no_project();
        let response = roundtrip(&mut app, Request::without_params(1, method));
        if let Some(error) = response.error {
            assert_ne!(
                error.kind(),
                ErrorKind::Busy,
                "{method} is allowlisted but was `busy`-gated with no active project: {error:?}"
            );
        }
    }
}
