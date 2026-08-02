//! `master.summary` / `master.set_volume` over the control endpoint
//! (ba doc #273, todo #1226).
//!
//! The master bus is the app's final summing stage and was reachable
//! only from the GUI: a mix driven over the control API could be
//! balanced correctly and still have nowhere to put a limiter. These
//! tests pin the read view (including the insert chain), the
//! exactly-one-of unit selection, range rejection, undoability, and the
//! `busy` answer when no project is open.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioCommand, AudioEvent};
use resonance_control::methods::master::MasterSummary;
use resonance_control::{ErrorKind, MutationAck, Request, Response};

fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    // Undo history only records once the project has a path.
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-master-test.rprj"));
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

fn call(app: &mut Resonance, method: &str, params: serde_json::Value) -> Response {
    roundtrip(app, Request::new(1, method, &params).expect("params serialize"))
}

fn summary(app: &mut Resonance) -> MasterSummary {
    roundtrip(app, Request::without_params(99, "master.summary"))
        .result()
        .expect("master.summary succeeds")
}

#[test]
fn summary_reports_the_real_master_state() {
    let mut app = app();
    let view = summary(&mut app);
    assert!((view.volume_db - 0.0).abs() < 1e-6, "fresh master is 0 dB");
    assert!((view.volume - 1.0).abs() < 1e-6, "0 dB is unity gain");
    assert!(!view.fx_bypassed);
    assert!(view.plugins.is_empty(), "a fresh master carries no inserts");
}

#[test]
fn summary_reports_the_master_insert_chain() {
    let mut app = app();
    // The engine echoes each master plugin as it instantiates it.
    app.test_apply_engine_event(AudioEvent::MasterPluginAdded {
        instance_id: 7,
        plugin_name: "Resonance Mastering".to_owned(),
        clap_plugin_id: "com.resonance.mastering".to_owned(),
        clap_file_path: "/plugins/mastering.clap".to_owned(),
        params: Vec::new(),
        has_gui: false,
    });
    app.test_apply_engine_event(AudioEvent::MasterFxBypassChanged { bypassed: true });

    let view = summary(&mut app);
    assert_eq!(view.plugins.len(), 1);
    assert_eq!(view.plugins[0].slot, 0);
    assert_eq!(view.plugins[0].plugin_id, "com.resonance.mastering");
    assert_eq!(view.plugins[0].name, "Resonance Mastering");
    assert!(view.fx_bypassed);
}

#[test]
fn set_volume_db_is_reflected_in_the_next_summary_and_reaches_the_engine() {
    let mut app = app();
    let rx = app.test_capture_engine();

    let ack: MutationAck = call(&mut app, "master.set_volume", serde_json::json!({"volume_db": -3.0}))
        .result()
        .expect("master.set_volume succeeds");
    assert!(ack.revision > 0);

    let view = summary(&mut app);
    assert!((view.volume_db - -3.0).abs() < 1e-4, "{}", view.volume_db);
    let expected = 10f32.powf(-3.0 / 20.0);
    assert!((view.volume - expected).abs() < 1e-4, "{}", view.volume);

    // The engine gets linear gain, as it always has.
    let mut gains = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        if let AudioCommand::SetMasterVolume { volume } = cmd {
            gains.push(volume);
        }
    }
    assert_eq!(gains.len(), 1, "one SetMasterVolume reached the engine");
    assert!((gains[0] - expected).abs() < 1e-4, "{}", gains[0]);
}

#[test]
fn set_volume_accepts_the_linear_form_too() {
    let mut app = app();
    let _: MutationAck = call(&mut app, "master.set_volume", serde_json::json!({"volume": 0.5}))
        .result()
        .expect("linear form succeeds");
    let view = summary(&mut app);
    assert!((view.volume_db - -6.0206).abs() < 1e-3, "{}", view.volume_db);
    assert!((view.volume - 0.5).abs() < 1e-4, "{}", view.volume);
}

#[test]
fn both_units_or_neither_is_invalid_params() {
    let mut app = app();
    for params in [
        serde_json::json!({"volume": 0.5, "volume_db": -6.0}),
        serde_json::json!({}),
    ] {
        let error = call(&mut app, "master.set_volume", params.clone())
            .error
            .unwrap_or_else(|| panic!("{params} should be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for {params}");
        assert!(
            error.message.contains("exactly one"),
            "the error must say which unit to pick: {}",
            error.message
        );
    }
    assert!(summary(&mut app).volume_db.abs() < 1e-6, "fader untouched");
}

#[test]
fn out_of_range_levels_are_rejected() {
    let mut app = app();
    for params in [
        serde_json::json!({"volume_db": 12.0}),
        serde_json::json!({"volume_db": -75.0}),
        serde_json::json!({"volume": 4.0}),
        serde_json::json!({"volume": -0.5}),
    ] {
        let error = call(&mut app, "master.set_volume", params.clone())
            .error
            .unwrap_or_else(|| panic!("{params} should be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for {params}");
    }
    assert!(summary(&mut app).volume_db.abs() < 1e-6, "fader untouched");
}

#[test]
fn the_master_fader_move_is_undoable() {
    let mut app = app();
    let _: MutationAck = call(&mut app, "master.set_volume", serde_json::json!({"volume_db": -5.0}))
        .result()
        .expect("succeeds");
    assert!((summary(&mut app).volume_db - -5.0).abs() < 1e-4);

    let _ = app.update(Message::Undo);
    assert!(
        summary(&mut app).volume_db.abs() < 1e-4,
        "undo should restore the master fader to 0 dB"
    );
}

/// `master.summary` reads only, but it describes the open project's
/// master — with no project the honest answer is `busy`, not a default.
#[test]
fn master_methods_need_an_open_project() {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(false);

    for (method, params) in [
        ("master.summary", serde_json::json!({})),
        ("master.set_volume", serde_json::json!({"volume_db": -3.0})),
    ] {
        let error = call(&mut app, method, params)
            .error
            .unwrap_or_else(|| panic!("{method} should be busy without a project"));
        assert_eq!(error.kind(), ErrorKind::Busy, "for {method}");
    }
}

#[test]
fn both_master_methods_are_advertised_in_the_handshake() {
    let capabilities = resonance_control::methods::capabilities();
    assert!(capabilities.contains(&"master.summary"));
    assert!(capabilities.contains(&"master.set_volume"));
}
