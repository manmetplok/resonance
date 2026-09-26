//! `master.summary` / `master.set_volume` over the control endpoint
//! (ba doc #273, todo #1226).
//!
//! The master bus is the app's final summing stage and was reachable
//! only from the GUI: a mix driven over the control API could be
//! balanced correctly and still have nowhere to put a limiter. These
//! tests pin the read view (including the insert chain), the
//! exactly-one-of unit selection, range rejection, undoability, and the
//! `busy` answer when no project is open.

use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::{AudioCommand, AudioEvent, ScannedPlugin};
use resonance_control::methods::master::MasterSummary;
use resonance_control::{ErrorKind, MutationAck, Request};
use crate::common::{call, roundtrip};

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    // Undo history only records once the project has a path.
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-master-test.rprj"));
    app
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
        has_sidechain_input: false,
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
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
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
fn every_master_method_is_advertised_in_the_handshake() {
    let capabilities = resonance_control::methods::capabilities();
    for method in [
        "master.summary",
        "master.set_volume",
        "master.add_effect",
        "master.remove_effect",
        "master.move_effect",
        "master.set_fx_bypass",
        "master.plugin_params",
        "master.set_plugin_param",
    ] {
        assert!(capabilities.contains(&method), "{method} missing");
    }
}

// ---------------------------------------------------------------------------
// The master insert chain (todo #1227)
// ---------------------------------------------------------------------------

fn seed_plugins(app: &mut Resonance) {
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![
            ScannedPlugin {
                clap_file_path: "/plugins/mastering.clap".to_owned(),
                clap_plugin_id: "com.resonance.mastering".to_owned(),
                name: "Resonance Mastering".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
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
            ScannedPlugin {
                clap_file_path: "/plugins/wavetable.clap".to_owned(),
                clap_plugin_id: "com.resonance.wavetable".to_owned(),
                name: "Resonance Wavetable".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: true,
            ..Default::default()
},
        ],
    });
}

/// The instance id the app allocated on the most recent master add. The
/// control path mirrors the slot immediately, so the engine's echo has
/// to carry the same id or it would land as a second plugin.
fn hinted(rx: &crossbeam_channel::Receiver<AudioCommand>) -> u64 {
    std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|c| match c {
            AudioCommand::AddPluginToMaster { id, .. } => Some(id),
            _ => None,
        })
        .expect("an AddPluginToMaster reached the engine")
}

/// Mirror the engine's `PluginAdded` echo for the master chain, the way
/// the real engine does after `AudioCommand::AddPluginToMaster`.
fn echo_master_plugin(app: &mut Resonance, instance_id: u64, clap_plugin_id: &str, name: &str) {
    app.test_apply_engine_event(AudioEvent::MasterPluginAdded {
        instance_id,
        plugin_name: name.to_owned(),
        clap_plugin_id: clap_plugin_id.to_owned(),
        clap_file_path: format!("/plugins/{clap_plugin_id}.clap"),
        params: Vec::new(),
        has_gui: false,
        has_sidechain_input: false,
    });
}

#[test]
fn add_effect_puts_the_plugin_on_the_master_chain() {
    let mut app = app();
    seed_plugins(&mut app);
    let rx = app.test_capture_engine();

    let _: MutationAck = call(
        &mut app,
        "master.add_effect",
        serde_json::json!({"plugin_id": "com.resonance.mastering"}),
    )
    .result()
    .expect("master.add_effect succeeds");

    // The plugin is on the chain in the same cycle as the reply — the
    // engine echo only fills in the parameter list (todo #1234's
    // read-your-writes rule, now shared by all three surfaces).
    let view = summary(&mut app);
    assert_eq!(view.plugins.len(), 1);
    assert_eq!(view.plugins[0].plugin_id, "com.resonance.mastering");
    assert_eq!(view.plugins[0].slot, 0);

    let instance_id = hinted(&rx);
    echo_master_plugin(
        &mut app,
        instance_id,
        "com.resonance.mastering",
        "Resonance Mastering",
    );
    let view = summary(&mut app);
    assert_eq!(view.plugins.len(), 1, "the echo fills the slot, not a second one");
}

#[test]
fn add_effect_refuses_instruments_and_unknown_ids() {
    let mut app = app();
    seed_plugins(&mut app);

    let error = call(
        &mut app,
        "master.add_effect",
        serde_json::json!({"plugin_id": "com.resonance.wavetable"}),
    )
    .error
    .expect("an instrument on the master is refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(
        error.message.contains("instrument"),
        "the error must say why: {}",
        error.message
    );

    let error = call(
        &mut app,
        "master.add_effect",
        serde_json::json!({"plugin_id": "com.example.nope"}),
    )
    .error
    .expect("an unknown id is refused");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(
        error.message.contains("com.resonance.mastering"),
        "the error must list valid ids: {}",
        error.message
    );

    assert!(summary(&mut app).plugins.is_empty());
}

#[test]
fn remove_effect_by_slot_leaves_the_other_one_renumbered() {
    let mut app = app();
    seed_plugins(&mut app);
    echo_master_plugin(&mut app, 1, "com.resonance.eq", "Resonance EQ");
    echo_master_plugin(&mut app, 2, "com.resonance.mastering", "Resonance Mastering");
    assert_eq!(summary(&mut app).plugins.len(), 2);

    let rx = app.test_capture_engine();
    let _: MutationAck = call(&mut app, "master.remove_effect", serde_json::json!({"slot": 0}))
        .result()
        .expect("master.remove_effect succeeds");
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok())
            .any(|c| matches!(c, AudioCommand::RemovePluginFromMaster { instance_id } if instance_id == 1)),
        "the engine must be told to unload the EQ instance"
    );

    app.test_apply_engine_event(AudioEvent::MasterPluginRemoved { instance_id: 1 });
    let view = summary(&mut app);
    assert_eq!(view.plugins.len(), 1);
    assert_eq!(view.plugins[0].plugin_id, "com.resonance.mastering");
    assert_eq!(view.plugins[0].slot, 0, "the survivor renumbers to slot 0");
}

#[test]
fn remove_effect_by_plugin_id_and_occurrence_targets_the_right_instance() {
    let mut app = app();
    seed_plugins(&mut app);
    echo_master_plugin(&mut app, 1, "com.resonance.eq", "Resonance EQ");
    echo_master_plugin(&mut app, 2, "com.resonance.eq", "Resonance EQ");

    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "master.remove_effect",
        serde_json::json!({"plugin_id": "com.resonance.eq", "occurrence": 1}),
    )
    .result()
    .expect("succeeds");
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok())
            .any(|c| matches!(c, AudioCommand::RemovePluginFromMaster { instance_id } if instance_id == 2)),
        "occurrence 1 is the SECOND EQ (instance 2)"
    );
}

#[test]
fn remove_effect_addressing_must_be_unambiguous() {
    let mut app = app();
    seed_plugins(&mut app);
    echo_master_plugin(&mut app, 1, "com.resonance.eq", "Resonance EQ");

    for params in [
        serde_json::json!({}),
        serde_json::json!({"slot": 0, "plugin_id": "com.resonance.eq"}),
    ] {
        let error = call(&mut app, "master.remove_effect", params.clone())
            .error
            .unwrap_or_else(|| panic!("{params} should be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for {params}");
    }

    // Addressing something that is not there is not_found, and lists the
    // chain so the caller can correct itself.
    let error = call(&mut app, "master.remove_effect", serde_json::json!({"slot": 5}))
        .error
        .expect("missing slot rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("com.resonance.eq"), "{}", error.message);
}

#[test]
fn set_fx_bypass_is_idempotent_and_reflected_in_the_summary() {
    let mut app = app();
    assert!(!summary(&mut app).fx_bypassed);

    let _: MutationAck = call(
        &mut app,
        "master.set_fx_bypass",
        serde_json::json!({"bypassed": true}),
    )
    .result()
    .expect("succeeds");
    assert!(summary(&mut app).fx_bypassed);

    // Setting the state it is already in is a no-op: no toggle back, and
    // no new committed revision.
    let before = app.revision();
    let _: MutationAck = call(
        &mut app,
        "master.set_fx_bypass",
        serde_json::json!({"bypassed": true}),
    )
    .result()
    .expect("succeeds");
    assert!(summary(&mut app).fx_bypassed, "must not have toggled back");
    assert_eq!(app.revision(), before, "a no-op records nothing");

    let _: MutationAck = call(
        &mut app,
        "master.set_fx_bypass",
        serde_json::json!({"bypassed": false}),
    )
    .result()
    .expect("succeeds");
    assert!(!summary(&mut app).fx_bypassed);
}

/// What the control API puts on the master must still be there after a
/// save + reload. Persistence itself is pre-existing
/// (`ProjectState.master_plugins` / `master_volume` /
/// `master_fx_bypassed`); this pins that a chain built remotely lands in
/// the serialized project rather than staying runtime-only.
#[test]
fn master_state_built_over_the_control_api_is_persisted() {
    let mut app = app();
    seed_plugins(&mut app);
    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "master.add_effect",
        serde_json::json!({"plugin_id": "com.resonance.mastering"}),
    )
    .result()
    .expect("succeeds");
    let instance_id = hinted(&rx);
    echo_master_plugin(
        &mut app,
        instance_id,
        "com.resonance.mastering",
        "Resonance Mastering",
    );
    let _: MutationAck = call(&mut app, "master.set_volume", serde_json::json!({"volume_db": -2.5}))
        .result()
        .expect("succeeds");
    let _: MutationAck = call(
        &mut app,
        "master.set_fx_bypass",
        serde_json::json!({"bypassed": true}),
    )
    .result()
    .expect("succeeds");

    let file = app.test_build_project_file();
    assert!((file.master_volume - -2.5).abs() < 1e-4, "{}", file.master_volume);
    assert!(file.master_fx_bypassed);
    let ids: Vec<&str> = file
        .master_plugins
        .iter()
        .map(|p| p.clap_plugin_id.as_str())
        .collect();
    assert_eq!(ids, vec!["com.resonance.mastering"]);
}

/// Adding a master plugin goes onto the undo stack, and (A-13h) its undo
/// removes that one instance on the diff path — no `ClearAll`, no other
/// plugin re-instantiated.
#[test]
fn chain_edits_are_recorded_on_the_undo_stack() {
    let mut app = app();
    seed_plugins(&mut app);
    let before = app.revision();

    let rx = app.test_capture_engine();
    let _: MutationAck = call(
        &mut app,
        "master.add_effect",
        serde_json::json!({"plugin_id": "com.resonance.mastering"}),
    )
    .result()
    .expect("succeeds");
    assert_eq!(app.revision(), before + 1, "the add is a committed edit");
    let instance_id = hinted(&rx);
    echo_master_plugin(
        &mut app,
        instance_id,
        "com.resonance.mastering",
        "Resonance Mastering",
    );
    assert_eq!(summary(&mut app).plugins.len(), 1);

    let rx = app.test_capture_engine();
    let _ = app.update(Message::Undo);
    let cmds: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "the add undoes on the diff path: {cmds:?}"
    );
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            AudioCommand::RemovePluginFromMaster { instance_id: id } if *id == instance_id
        )),
        "undo must remove the added instance: {cmds:?}"
    );
    assert!(summary(&mut app).plugins.is_empty());
}
