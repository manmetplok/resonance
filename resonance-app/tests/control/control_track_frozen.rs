//! Frozen-track rejection for the `track.*` plugin-chain methods.
//!
//! The frozen-input gate (`frozen_input_edit_target`, ba todo #576)
//! swallows plugin edits aimed at a frozen track — no mutation, no undo —
//! so a control handler that dispatched one and acked was falsely
//! reporting an edit that never happened (`track.add_effect` even
//! fabricated a `{slot, occurrence}` reply for a plugin never added).
//! These assert every chain-editing method pre-checks the freeze and
//! answers `busy`, the same rule `notes.*` applies to note edits, and
//! that the same calls still go through once the track is unfrozen.

use resonance_app::state::{FreezeStatus, ViewMode};
use resonance_app::{Resonance};
use resonance_audio::types::{AudioEvent, ParamInfo, ScannedPlugin, TrackType};
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};
use resonance_control::methods::track::PluginParamsView;
use resonance_control::ErrorKind;
use crate::common::call;

const SR: u32 = 48_000;
const TRACK: u64 = 1;
/// Sidechain key source for `track.set_sidechain`.
const SOURCE: u64 = 2;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-track-frozen.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_add_track(SOURCE, TrackType::Instrument);
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
            ScannedPlugin {
                clap_file_path: "/plugins/reverb.clap".to_owned(),
                clap_plugin_id: "com.resonance.reverb".to_owned(),
                name: "Resonance Reverb".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
                ..Default::default()
            },
        ],
    });
    // Chain: wavetable (instrument, slot 0), EQ (slot 1, has a param and
    // a key port so set_plugin_param / set_sidechain can address it),
    // reverb (slot 2).
    echo_plugin(&mut app, 10, "com.resonance.wavetable", false);
    echo_plugin(&mut app, 11, "com.resonance.eq", true);
    echo_plugin(&mut app, 12, "com.resonance.reverb", false);
    app
}

/// Mirror the engine echo that mounts a plugin on the track, carrying
/// one parameter so `track.set_plugin_param` can address it.
fn echo_plugin(app: &mut Resonance, instance_id: u64, plugin_id: &str, keyable: bool) {
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id,
        plugin_name: plugin_id.to_owned(),
        clap_plugin_id: plugin_id.to_owned(),
        clap_file_path: format!("/plugins/{plugin_id}.clap"),
        params: vec![ParamInfo {
            id: 1,
            name: "Gain".to_owned(),
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.5,
            current_value: 0.5,
            ..Default::default()
        }],
        has_gui: false,
        has_sidechain_input: keyable,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
}

fn freeze(app: &mut Resonance, status: FreezeCacheStatus) {
    let cache = FreezeCacheRef::new("freeze_1.wav".to_owned(), SR, 32, 1, status);
    let status = match status {
        FreezeCacheStatus::Frozen => FreezeStatus::Frozen { cache_ref: cache },
        _ => FreezeStatus::Stale { cache_ref: cache },
    };
    app.test_set_freeze_status(TRACK, status);
}

fn chain_ids(app: &mut Resonance) -> Vec<String> {
    let view: PluginParamsView = call(
        app,
        "track.plugin_params",
        serde_json::json!({ "track_id": TRACK }),
    )
    .result()
    .expect("track.plugin_params succeeds");
    view.plugins.iter().map(|p| p.plugin_id.clone()).collect()
}

/// Every chain-editing `track.*` method whose message the frozen-input
/// gate swallows, with params that are valid on the unfrozen fixture —
/// the positive test below runs the same table.
fn gated_calls() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        (
            "track.set_plugin_param",
            serde_json::json!({ "track_id": TRACK, "plugin_id": "com.resonance.eq", "param": "Gain", "value": 0.25 }),
        ),
        (
            "track.add_effect",
            serde_json::json!({ "track_id": TRACK, "plugin_id": "com.resonance.reverb" }),
        ),
        (
            "track.add_instrument",
            serde_json::json!({ "track_id": TRACK, "plugin_id": "com.resonance.wavetable" }),
        ),
        (
            "track.remove_effect",
            serde_json::json!({ "track_id": TRACK, "slot": 2 }),
        ),
        (
            "track.move_effect",
            serde_json::json!({ "track_id": TRACK, "slot": 1, "to_slot": 2 }),
        ),
        (
            "track.replace_effect",
            serde_json::json!({ "track_id": TRACK, "slot": 2, "new_plugin_id": "com.resonance.eq" }),
        ),
        (
            "track.set_plugin_bypass",
            serde_json::json!({ "track_id": TRACK, "plugin_id": "com.resonance.eq", "bypassed": true }),
        ),
        (
            "track.set_fx_bypass",
            serde_json::json!({ "track_id": TRACK, "bypassed": true }),
        ),
        (
            "track.set_sidechain",
            serde_json::json!({ "track_id": TRACK, "source_track_id": SOURCE }),
        ),
        (
            "track.clear_sidechain",
            serde_json::json!({ "track_id": TRACK }),
        ),
        (
            "track.load_plugin_preset",
            serde_json::json!({ "track_id": TRACK, "plugin_id": "com.resonance.eq", "preset": "Warm" }),
        ),
    ]
}

#[test]
fn chain_edits_on_a_frozen_track_are_rejected_busy_with_no_change() {
    let mut app = app();
    freeze(&mut app, FreezeCacheStatus::Frozen);

    let revision_before = app.revision();
    let chain_before = chain_ids(&mut app);

    // Each method must be rejected with a stable `busy` kind — not
    // swallowed by the frozen-input gate and falsely acked — and must
    // not touch the chain or bump the revision.
    for (method, params) in gated_calls() {
        let response = call(&mut app, method, params);
        let error = response
            .error
            .unwrap_or_else(|| panic!("{method} on a frozen track must be rejected"));
        assert_eq!(error.kind(), ErrorKind::Busy, "{method}");
        assert!(error.message.contains("frozen"), "{method}: {}", error.message);
    }

    // Nothing changed: same chain, same revision — and the freeze is
    // still Frozen, not Stale. A rejected edit must never have reached
    // the gate (which invalidates the cache on its way to dropping it).
    assert_eq!(chain_ids(&mut app), chain_before);
    assert_eq!(app.revision(), revision_before);
    assert!(
        matches!(app.test_freeze_status(TRACK), FreezeStatus::Frozen { .. }),
        "the pre-check must fire before dispatch, leaving the cache valid"
    );
}

#[test]
fn a_stale_freeze_rejects_chain_edits_too() {
    // `Stale` still plays from the cache, so its inputs are just as
    // read-only as `Frozen` — same rule the gate applies.
    let mut app = app();
    freeze(&mut app, FreezeCacheStatus::Stale);

    let revision_before = app.revision();
    let error = call(
        &mut app,
        "track.set_plugin_param",
        serde_json::json!({ "track_id": TRACK, "plugin_id": "com.resonance.eq", "param": "Gain", "value": 0.25 }),
    )
    .error
    .expect("a stale-frozen track still rejects chain edits");
    assert_eq!(error.kind(), ErrorKind::Busy);
    assert!(error.message.contains("frozen"), "{}", error.message);
    assert_eq!(app.revision(), revision_before);
}

#[test]
fn the_same_edits_on_an_unfrozen_track_still_succeed() {
    let mut app = app();
    let revision_before = app.revision();

    for (method, params) in gated_calls() {
        let response = call(&mut app, method, params);
        if method == "track.load_plugin_preset" {
            // No preset named "Warm" exists in the hermetic fixture, so
            // this one cannot ack — but it must fail on the PRESET, not
            // on a freeze that is not there.
            let error = response.error.expect("unknown preset is rejected");
            assert_ne!(error.kind(), ErrorKind::Busy, "{}", error.message);
            assert!(!error.message.contains("frozen"), "{}", error.message);
            continue;
        }
        assert!(
            response.error.is_none(),
            "{method} must succeed on an unfrozen track: {:?}",
            response.error
        );
    }
    assert!(
        app.revision() > revision_before,
        "unfrozen edits are committed and bump the revision"
    );
}
