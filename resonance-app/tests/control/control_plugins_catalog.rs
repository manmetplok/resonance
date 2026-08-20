//! `plugins.catalog`, the renamed installed-plugin catalog (ba doc
//! #273, todo #1236).
//!
//! It used to be `track.plugins`, which read as "the plugins on a
//! track". A field agent believed it from the name, concluded there was
//! no way to inspect a track's chain, and reported that as a missing
//! feature — while `track.plugin_params` had reported the real loaded
//! chain, with full parameter metadata, all along.
//!
//! The `track.plugins` alias that #1236 kept for one release was removed
//! in todo #1240, so `plugins.catalog` is now the only spelling — the
//! test below pins that the old name is genuinely gone rather than
//! silently still answering.
//!
//! The other property asserted here is that the catalog is answerable
//! with NO project open — it reads the startup scanner's results, which
//! no project owns, so the mutation gate must not turn it into `busy`.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::{AudioEvent, ScannedPlugin};
use resonance_control::methods::plugins::{PluginCatalog, CATALOG};
use resonance_control::{ErrorKind, Request, Response};

/// The name the catalog carried before #1236. Spelled out rather than
/// imported: the constant it came from no longer exists, and the point
/// of the test below is that nothing answers to this string.
const REMOVED_ALIAS: &str = "track.plugins";

/// An app with a scanned catalog but NO active project.
fn app_without_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![
            ScannedPlugin {
                clap_file_path: "/plugins/wavetable.clap".to_owned(),
                clap_plugin_id: "com.resonance.wavetable".to_owned(),
                name: "Resonance Wavetable".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: true,
            ..Default::default()
},
            ScannedPlugin {
                clap_file_path: "/plugins/eq.clap".to_owned(),
                clap_plugin_id: "com.resonance.eq".to_owned(),
                name: "Resonance EQ".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
            ..Default::default()
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
fn the_removed_alias_no_longer_answers() {
    // The deprecation window closed in todo #1240. A client still
    // spelling it the old way must get a clean `unsupported`, not a
    // silent success — that is the whole point of removing it rather
    // than leaving it to rot.
    let mut app = app_without_project();
    let error = catalog(&mut app, REMOVED_ALIAS)
        .error
        .expect("the alias was removed, so nothing should answer to it");
    assert_eq!(error.kind(), ErrorKind::Unsupported);
}

#[test]
fn only_the_current_name_is_advertised_in_the_hello_capabilities() {
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
        "the current name must be advertised"
    );
    assert!(
        !hello.capabilities.iter().any(|m| m == REMOVED_ALIAS),
        "the removed alias must be gone from capabilities too, or \
         control.hello advertises a name the app no longer answers"
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
    assert_eq!(app.revision(), before);
    assert!(!app.is_dirty());
}
