//! `AudioCommand::ProbeChain` (warmth-width-depth.md §7.3, W3): the
//! engine clones a chain from its live instances' state, probes the
//! clones on a worker, and leaves the live instances untouched.
//!
//! The empty-chain case runs everywhere. The mastering-saturator case
//! needs the real plugin binary — `target/bundled/resonance-mastering.clap`
//! (scripts/bundle.sh) or the debug cdylib `libresonance_mastering.so`
//! (`cargo build -p resonance-mastering`) — and skips without one, the
//! same pattern as `clap_all_notes_off.rs`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::types::*;

const PROBE_ID: u64 = 7_311;

fn spec() -> ProbeSpec {
    ProbeSpec {
        freq_hz: 1_000.0,
        level_dbfs: -12.0,
        imd: true,
    }
}

/// Dispatch a probe and wait for its terminal event.
fn probe(harness: &mut EngineHandlerHarness, stages: Vec<ProbeStage>) -> AudioEvent {
    harness.dispatch(AudioCommand::ProbeChain {
        probe_id: PROBE_ID,
        stages,
        spec: spec(),
    });
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        for event in harness.drain_events() {
            if matches!(
                event,
                AudioEvent::ChainProbed { .. } | AudioEvent::ChainProbeError { .. }
            ) {
                return event;
            }
        }
        assert!(Instant::now() < deadline, "the probe never answered");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn report(event: AudioEvent) -> ChainProbeReport {
    match event {
        AudioEvent::ChainProbed { probe_id, report } => {
            assert_eq!(probe_id, PROBE_ID, "the token is echoed");
            report
        }
        other => panic!("expected ChainProbed, got {other:?}"),
    }
}

#[test]
fn an_empty_chain_is_a_straight_wire() {
    let mut harness = EngineHandlerHarness::new();
    let r = report(probe(&mut harness, Vec::new()));
    assert!(r.stages.is_empty());
    assert!(r.harmonics.thd_pct < 1e-3, "{:?}", r.harmonics);
    assert!((r.harmonics.fundamental_dbfs - -12.0).abs() < 0.01);
    assert!(r.harmonics.aliasing_floor_dbc < -120.0);
    assert!(r.imd_pct.unwrap() < 1e-3);
    assert_eq!(r.latency_samples, 0);
}

#[test]
fn a_stage_whose_live_instance_is_gone_fails_the_probe() {
    let mut harness = EngineHandlerHarness::new();
    let event = probe(
        &mut harness,
        vec![ProbeStage {
            instance_id: 4_040,
            clap_file_path: "/nowhere/x.clap".into(),
            clap_plugin_id: "com.example.x".into(),
        }],
    );
    match event {
        AudioEvent::ChainProbeError { probe_id, message } => {
            assert_eq!(probe_id, PROBE_ID);
            assert!(message.contains("4040"), "{message}");
        }
        other => panic!("expected ChainProbeError, got {other:?}"),
    }
}

fn mastering_binary() -> Option<PathBuf> {
    [
        "target/bundled/resonance-mastering.clap",
        "../target/bundled/resonance-mastering.clap",
        "target/debug/libresonance_mastering.so",
        "../target/debug/libresonance_mastering.so",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
}

/// Set `key` to `to` wherever it appears in a JSON state.
fn set_key(value: &mut serde_json::Value, key: &str, to: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            let mut found = false;
            for (k, v) in map.iter_mut() {
                if k == key {
                    *v = to.clone();
                    found = true;
                } else {
                    found |= set_key(v, key, to);
                }
            }
            found
        }
        serde_json::Value::Array(items) => {
            items.iter_mut().fold(false, |found, v| set_key(v, key, to) | found)
        }
        _ => false,
    }
}

#[test]
fn the_mastering_saturator_in_tape_mode_shows_h2_and_the_live_instance_is_untouched() {
    let Some(path) = mastering_binary() else {
        eprintln!(
            "[skip] no resonance-mastering binary (scripts/bundle.sh, or cargo build -p \
             resonance-mastering)"
        );
        return;
    };
    let path = path.canonicalize().unwrap().to_string_lossy().into_owned();
    let mut harness = EngineHandlerHarness::new();
    harness.add_track(1, None);
    harness.add_plugin(1, path.clone(), "com.resonance.mastering".into(), 100);
    harness.drain_events();

    // Configure the LIVE instance: saturator on, full Tape character.
    let before = {
        let plugins = harness.shared().plugins();
        let mut live = plugins.get(&100).expect("the plugin loaded").lock();
        let saved = live.0.save_state().expect("the plugin saves state");
        let mut json: serde_json::Value = serde_json::from_slice(&saved).expect("JSON state");
        for (key, value) in [
            ("sat_on", serde_json::json!(1.0)),
            ("sat_character", serde_json::json!(1.0)),
            ("sat_drive", serde_json::json!(12.0)),
        ] {
            assert!(set_key(&mut json, key, &value), "state has no `{key}`: {json}");
        }
        assert!(live.0.reload_with_state(&serde_json::to_vec(&json).unwrap()));
        live.0.save_state().unwrap()
    };

    let r = report(probe(
        &mut harness,
        vec![ProbeStage {
            instance_id: 100,
            clap_file_path: path,
            clap_plugin_id: "com.resonance.mastering".into(),
        }],
    ));
    assert_eq!(r.stages.len(), 1);
    assert!(r.stages[0].state_copied, "the clone carries the live state");
    let h2 = r.harmonics.h[0].unwrap();
    assert!(h2 > -60.0, "Tape character adds H2: {h2} dBc ({:?})", r.harmonics);
    assert!(r.harmonics.thd_pct > 0.1, "{:?}", r.harmonics);

    // The probe only READ the live instance.
    let plugins = harness.shared().plugins();
    let after = plugins.get(&100).unwrap().lock().0.save_state().unwrap();
    assert_eq!(before, after, "the live plugin's state is untouched");
    assert_eq!(harness.plugin_instance_count(), 1, "the clone is not in the live map");
}
