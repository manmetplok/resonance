//! `plugins.catalog`, the renamed installed-plugin catalog (ba doc
//! #273, todo #1236).
//!
//! It used to be `track.plugins`, which read as "the plugins on a
//! track". A field agent believed it from the name, concluded there was
//! no way to inspect a track's chain, and reported that as a missing
//! feature — while `track.plugin_params` had reported the real loaded
//! chain, with full parameter metadata, all along.
//!
//! Two properties are asserted here: the deprecated alias still answers
//! identically (it is kept for one release), and the catalog is
//! answerable with NO project open — it reads the startup scanner's
//! results, which no project owns, so the mutation gate must not turn it
//! into `busy`.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioEvent, ScannedPlugin};
use resonance_control::methods::plugins::{PluginCatalog, CATALOG};
use resonance_control::{ErrorKind, Request, Response};

#[allow(deprecated)]
const ALIAS: &str = resonance_control::methods::plugins::PLUGINS_DEPRECATED_ALIAS;

/// An app with a scanned catalog but NO active project.
fn app_without_project() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new_for_test();
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![
            ScannedPlugin {
                clap_file_path: "/plugins/wavetable.clap".to_owned(),
                clap_plugin_id: "com.resonance.wavetable".to_owned(),
                name: "Resonance Wavetable".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: true,
            },
            ScannedPlugin {
                clap_file_path: "/plugins/eq.clap".to_owned(),
                clap_plugin_id: "com.resonance.eq".to_owned(),
                name: "Resonance EQ".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
            },
        ],
    });
    app
}

fn roundtrip(app: &mut Resonance, req: Request) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request: req,
        reply,
    })));
    rx.try_recv().expect("one reply per request")
}

fn catalog(app: &mut Resonance, method: &str) -> Response {
    roundtrip(app, Request::without_params(1, method))
}

#[test]
fn the_catalog_answers_with_no_project_open() {
    // Nothing was opened or created; `a_mutating_method_is_still_gated_
    // with_no_project_open` proves the gate is live in this state.
    let mut app = app_without_project();
    let result: PluginCatalog = catalog(&mut app, CATALOG)
        .result()
        .expect("a pure catalog query must not need a project");
    assert_eq!(result.plugins.len(), 2);
    assert_eq!(result.plugins[0].id, "com.resonance.wavetable");
}

#[test]
fn a_mutating_method_is_still_gated_with_no_project_open() {
    // The counterpart to the test above: the gate is still there, the
    // catalog is simply exempt from it.
    let mut app = app_without_project();
    let error = roundtrip(
        &mut app,
        Request::new(1, "track.add", &serde_json::json!({"kind": "instrument"}))
            .expect("params serialize"),
    )
    .error
    .expect("a mutation with no project must be refused");
    assert_eq!(error.kind(), ErrorKind::Busy);
}

#[test]
fn the_deprecated_alias_returns_exactly_the_same_payload() {
    let mut app = app_without_project();
    let new_name: serde_json::Value = catalog(&mut app, CATALOG).result().expect("succeeds");
    let old_name: serde_json::Value = catalog(&mut app, ALIAS)
        .result()
        .expect("the alias is kept reachable for one release");
    assert_eq!(
        new_name, old_name,
        "one handler serves both names, so they cannot answer differently"
    );
}

#[test]
fn both_names_are_advertised_in_the_hello_capabilities() {
    let mut app = app_without_project();
    let hello: resonance_control::methods::control::HelloResult = roundtrip(
        &mut app,
        Request::new(
            1,
            "control.hello",
            &serde_json::json!({"protocol_version": resonance_control::PROTOCOL_VERSION}),
        )
        .expect("params serialize"),
    )
    .result()
    .expect("handshake succeeds");

    assert!(
        hello.capabilities.iter().any(|m| m == CATALOG),
        "the new name must be advertised"
    );
    assert!(
        hello.capabilities.iter().any(|m| m == ALIAS),
        "and so must the alias, or control.hello lies about what the app answers"
    );
    assert!(
        !hello.capabilities.iter().any(|m| m == "track.plugin_list"),
        "sanity: capabilities is not a wildcard"
    );
}

#[test]
fn the_catalog_reports_instrument_and_effect_kinds() {
    let mut app = app_without_project();
    let json: serde_json::Value = catalog(&mut app, CATALOG).result().expect("succeeds");
    assert_eq!(json["plugins"][0]["kind"], "instrument");
    assert_eq!(json["plugins"][1]["kind"], "effect");
}

#[test]
fn the_catalog_never_mutates() {
    let mut app = app_without_project();
    let before = app.revision();
    let _ = catalog(&mut app, CATALOG);
    let _ = catalog(&mut app, ALIAS);
    assert_eq!(app.revision(), before);
    assert!(!app.is_dirty());
}
