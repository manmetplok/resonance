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

// ---------------------------------------------------------------------------
// Clone teardown thread
// ---------------------------------------------------------------------------

mod teardown {
    use std::ffi::{c_char, c_void, CStr};
    use std::ptr;
    use std::sync::{Arc, Mutex};
    use std::thread::ThreadId;
    use std::time::{Duration, Instant};

    use clap_sys::ext::audio_ports::{
        clap_audio_port_info, clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS,
    };
    use clap_sys::host::clap_host;
    use clap_sys::id::clap_id;
    use clap_sys::plugin::clap_plugin;
    use clap_sys::process::clap_process;

    use resonance_audio::test_support::{
        EngineHandlerHarness, SyncClapInstance, __instance_from_raw_for_test,
    };
    use resonance_audio::types::*;

    /// Which thread ran each lifecycle call.
    type Log = Arc<Mutex<Vec<(&'static str, ThreadId)>>>;

    unsafe fn log<'a>(p: *const clap_plugin) -> &'a Log {
        &*((*p).plugin_data as *const Log)
    }
    unsafe fn record(p: *const clap_plugin, call: &'static str) {
        log(p).lock().unwrap().push((call, std::thread::current().id()));
    }
    unsafe extern "C" fn c_init(_: *const clap_plugin) -> bool {
        true
    }
    unsafe extern "C" fn c_destroy(p: *const clap_plugin) {
        record(p, "destroy");
    }
    unsafe extern "C" fn c_activate(_: *const clap_plugin, _: f64, _: u32, _: u32) -> bool {
        true
    }
    unsafe extern "C" fn c_deactivate(p: *const clap_plugin) {
        record(p, "deactivate");
    }
    unsafe extern "C" fn c_start(_: *const clap_plugin) -> bool {
        true
    }
    unsafe extern "C" fn c_stop(_: *const clap_plugin) {}
    unsafe extern "C" fn c_reset(_: *const clap_plugin) {}
    unsafe extern "C" fn c_main_thread(_: *const clap_plugin) {}
    unsafe extern "C" fn c_process(p: *const clap_plugin, _: *const clap_process) -> i32 {
        record(p, "process");
        1
    }
    unsafe extern "C" fn c_ports_count(_: *const clap_plugin, _: bool) -> u32 {
        1
    }
    unsafe extern "C" fn c_ports_get(
        _: *const clap_plugin,
        index: u32,
        _: bool,
        info: *mut clap_audio_port_info,
    ) -> bool {
        if index != 0 || info.is_null() {
            return false;
        }
        let out = &mut *info;
        out.id = 0 as clap_id;
        out.name = [0; 256];
        out.flags = 0;
        out.channel_count = 2;
        out.port_type = ptr::null();
        out.in_place_pair = 0;
        true
    }
    static PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
        count: Some(c_ports_count),
        get: Some(c_ports_get),
    };
    unsafe extern "C" fn c_get_extension(
        _: *const clap_plugin,
        id: *const c_char,
    ) -> *const c_void {
        if !id.is_null() && CStr::from_ptr(id) == CLAP_EXT_AUDIO_PORTS {
            return &PORTS as *const clap_plugin_audio_ports as *const c_void;
        }
        ptr::null()
    }

    fn logging_effect(calls: &Log) -> SyncClapInstance {
        let calls = Arc::clone(calls);
        let inst = __instance_from_raw_for_test(
            move |_host: *const clap_host| {
                let data = Box::into_raw(Box::new(calls));
                Box::into_raw(Box::new(clap_plugin {
                    desc: ptr::null(),
                    plugin_data: data as *mut c_void,
                    init: Some(c_init),
                    destroy: Some(c_destroy),
                    activate: Some(c_activate),
                    deactivate: Some(c_deactivate),
                    start_processing: Some(c_start),
                    stop_processing: Some(c_stop),
                    reset: Some(c_reset),
                    process: Some(c_process),
                    get_extension: Some(c_get_extension),
                    on_main_thread: Some(c_main_thread),
                })) as *const clap_plugin
            },
            48_000,
        )
        .expect("logging effect builds");
        SyncClapInstance(inst)
    }

    /// The clones are processed on the `probe-chain` worker, but CLAP's
    /// `deactivate` / `destroy` are main-thread calls — the engine
    /// thread here. The worker must hand them back rather than drop them.
    #[test]
    fn probe_clones_are_destroyed_on_the_engine_thread() {
        let calls: Log = Arc::default();
        let mut harness = EngineHandlerHarness::new();
        let engine_thread = std::thread::current().id();
        harness.probe_instances(
            9_001,
            vec![logging_effect(&calls), logging_effect(&calls)],
            ProbeSpec {
                freq_hz: 1_000.0,
                level_dbfs: -12.0,
                imd: false,
            },
        );
        // Wait for the report, then for the clones to come back.
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut reported = false;
        loop {
            reported |= harness
                .drain_events()
                .iter()
                .any(|e| matches!(e, AudioEvent::ChainProbed { probe_id: 9_001, .. }));
            harness.apply_worker_results();
            let destroyed = calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(call, _)| *call == "destroy")
                .count();
            if reported && destroyed == 2 {
                break;
            }
            assert!(Instant::now() < deadline, "the probe never finished: {calls:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
        let calls = calls.lock().unwrap();
        assert!(
            calls.iter().any(|(call, t)| *call == "process" && *t != engine_thread),
            "the clones are processed off the engine thread"
        );
        for (call, thread) in calls.iter().filter(|(c, _)| *c != "process") {
            assert_eq!(*thread, engine_thread, "{call} ran off the engine thread");
        }
    }
}
