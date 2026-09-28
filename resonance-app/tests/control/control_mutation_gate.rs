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

use resonance_app::message::{Message, ProjectIoMessage};
use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::AudioCommand;
use resonance_control::methods;
use resonance_control::{ErrorKind, Request};
use crate::common::roundtrip;

/// A fresh app with NO active project (boot state: the startup modal).
fn app_no_project() -> Resonance {
    // `Resonance::new_for_test_on(ViewMode::Arrange)` boots onto the startup project-picker modal, so
    // `has_active_project` is false — exactly the gated state under test.
    let (app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app
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
        // `external.devices` / `external.status` mutate nothing, but
        // they describe the OPEN project's hardware wiring, so with
        // nothing open the honest answer is `busy` rather than an empty
        // list that reads like "no external instruments".
        methods::external::METHODS,
        // `edit.status` reads only, but like `master.summary` it
        // describes the OPEN project's history (ba doc #273).
        methods::edit::METHODS,
        methods::section::METHODS,
        methods::harmony::METHODS,
        methods::generate::METHODS,
        methods::notes::METHODS,
        methods::vocal::METHODS,
        methods::render::METHODS,
        // `meter.*` mutates nothing, but it MEASURES the open project,
        // so with nothing open the honest answer is `busy` rather than a
        // measurement of silence that reads like a real one (todo
        // #1219).
        methods::meter::METHODS,
        // `arrangement.*` restructures the open project's timeline (ba
        // doc #275 P2).
        methods::arrangement::METHODS,
        // `global.list_events` reads only, but like `master.summary` it
        // describes the OPEN project — reporting the default 120 BPM 4/4
        // with nothing open would read like a real song's tempo map (ba
        // doc #286). The rest of `global.*` mutates outright.
        methods::global::METHODS,
        // `pool.list` reads only, but like `master.summary` it describes
        // the OPEN project — an empty asset list reads like "this project
        // has no samples", which is a different claim from "there is no
        // project". The rest of `pool.*` / `clip.*` mutates outright.
        methods::pool::METHODS,
        methods::clip::METHODS,
        // `reference.load` adds to the open project's reference list.
        methods::reference::METHODS,
        // `automation.lanes` reads only, but like `master.summary` it
        // describes the OPEN project's lanes — "no lanes" and "no
        // project" are different claims. `set_lane` mutates outright.
        methods::automation::METHODS,
    ] {
        methods.extend_from_slice(namespace);
    }
    // `track.plugin_params` lives in `track::METHODS` but is a read-only
    // query served by the `song::try_handle` block above the gate.
    // Everything else in `track::*` is a mutation. (`plugins.catalog`
    // moved out of this namespace entirely in todo #1236.)
    methods.retain(|m| *m != methods::track::PLUGIN_PARAMS);
    methods
}

/// The read-only / lifecycle allowlist: methods that legitimately run
/// with no active project and so must NOT be `busy`-gated.
fn allowlisted_methods() -> Vec<&'static str> {
    // `track.plugin_params` is the read-only member of the
    // otherwise-mutating `track::*` namespace. `plugins.*` — the
    // installed-plugin catalog — is read-only AND project-independent:
    // it reads the startup scanner's results, so gating it made a pure
    // catalog query answer `busy` with nothing open (todo #1236).
    let mut methods = vec!["control.hello", methods::track::PLUGIN_PARAMS];
    for namespace in [
        methods::song::METHODS,
        methods::project::METHODS,
        methods::plugins::METHODS,
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

/// A WAV mixdown (`io.bouncing`, started by the GUI bounce dialog or by
/// `render.mixdown`) drives the live plugin instances from a worker
/// thread, exactly like a bounce in place or a freeze. Until code review
/// UPD-06 it gated nothing: `transport.play` was accepted mid-export and
/// the live callback and the export interleaved `process()` on the same
/// CLAP instances. The engine refuses on its own now (MIX-02); the
/// control surface must answer `busy` rather than send a command the
/// engine will bounce.
#[test]
fn transport_play_is_busy_while_a_wav_mixdown_renders() {
    let (mut app, _task, cmd_rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::BouncePathSelected(Some(
        "/tmp/control-mutation-gate-mixdown.wav".to_owned(),
    ))));
    assert!(app.test_is_bouncing(), "the bounce dialog's path starts the mixdown");
    // Drain the `BounceToWav` the dialog sent.
    while cmd_rx.try_recv().is_ok() {}

    let response = roundtrip(&mut app, Request::without_params(1, "transport.play"));
    let error = response
        .error
        .expect("transport.play must not succeed while a mixdown renders");
    assert_eq!(error.kind(), ErrorKind::Busy, "got {error:?}");
    assert!(
        !cmd_rx.try_iter().any(|c| matches!(c, AudioCommand::Play)),
        "no Play may reach the engine while the export owns the plugins"
    );
}
