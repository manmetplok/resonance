use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use resonance_drums::articulation::{ARTICULATION_ALT, ARTICULATION_PRIMARY};
use resonance_drums::kit_loader::{PadMicChoices, DEFAULT_OVERHEAD_SETUP};
use resonance_drums::params::DrumParams;
use resonance_drums::{drum_map, DrumsExtraState, ResonanceDrums};
use resonance_plugin::plugin::ExtraStateSaver;
use resonance_plugin::ResonancePlugin;

/// save_state -> load_state round-trip preserves a kit path.
/// Exercises the main-thread path where the host calls save_state /
/// load_state on the owned plugin instance.
#[test]
fn state_roundtrip_preserves_kit_path() {
    let src = ResonanceDrums::new();
    *src.bridge.kit_path.lock() = Some(PathBuf::from("/some/kit/drum_samples.json"));

    let bytes = src.save_state();

    let mut dst = ResonanceDrums::new();
    assert!(dst.load_state(&bytes));
    let restored = dst.bridge.kit_path.lock().clone();
    assert_eq!(restored, Some(PathBuf::from("/some/kit/drum_samples.json")));
}

/// save_state with no kit followed by load_state clears any prior path.
#[test]
fn load_state_null_clears_kit_path() {
    let src = ResonanceDrums::new();
    let bytes = src.save_state(); // kit_path is None, serializes as null

    let mut dst = ResonanceDrums::new();
    // Pre-populate a stale path; load_state should clear it.
    *dst.bridge.kit_path.lock() = Some(PathBuf::from("/stale/path.json"));

    assert!(dst.load_state(&bytes));
    assert_eq!(*dst.bridge.kit_path.lock(), None);
}

type SaverBundle = (
    Arc<Mutex<Option<PathBuf>>>,
    Arc<Mutex<String>>,
    Arc<Mutex<[PadMicChoices; drum_map::NUM_PADS]>>,
    Arc<DrumParams>,
    DrumsExtraState,
);

/// Helper: build an empty saver storage bundle used by the saver tests.
fn make_saver_bundle(initial_path: Option<PathBuf>) -> SaverBundle {
    let kit_path = Arc::new(Mutex::new(initial_path));
    let overhead_setup_key = Arc::new(Mutex::new(DEFAULT_OVERHEAD_SETUP.to_string()));
    let pad_choices = Arc::new(Mutex::new(std::array::from_fn(|_| {
        PadMicChoices::default()
    })));
    // Articulations live in the params now (ba todo #1325), so the saver
    // reads them from there rather than from a mirror of its own.
    let params = Arc::new(DrumParams::default());
    let saver = DrumsExtraState {
        kit_path: kit_path.clone(),
        overhead_setup_key: overhead_setup_key.clone(),
        pad_choices: pad_choices.clone(),
        params: params.clone(),
        reload: None,
    };
    (kit_path, overhead_setup_key, pad_choices, params, saver)
}

/// Round-trip through the `ExtraStateSaver` interface directly. This
/// simulates what the CLAP bridge does when the plugin is in the audio
/// processor and the host asks for a state save — the owned plugin
/// isn't reachable, so the bridge talks to the cached saver instead.
/// This is exactly the path that used to silently drop kit_path at
/// project save time before the framework fix.
#[test]
fn extra_saver_roundtrip_active_path() {
    // Construct the saver the same way editor_factory / new() would,
    // holding shared arcs for each persisted field.
    let (_kp, _oh, _pc, _art, saver) =
        make_saver_bundle(Some(PathBuf::from("/active/path/drum_samples.json")));

    // Serialize — this is what clap_bridge::save() would do on the
    // plugin-is-None branch.
    let mut json = serde_json::json!({ "params": {} });
    for (k, v) in saver.save() {
        json.as_object_mut().unwrap().insert(k, v);
    }

    // New instance with a different shared storage — clear to start.
    let (restored_path, _, _, _, restored_saver) = make_saver_bundle(None);

    // Load from the serialized state.
    restored_saver.load(&json);

    assert_eq!(
        *restored_path.lock(),
        Some(PathBuf::from("/active/path/drum_samples.json")),
        "kit_path should round-trip through the saver"
    );
}

/// A loaded null kit_path through the saver clears previously stored path.
#[test]
fn extra_saver_null_clears_active_path() {
    let (kit_path, _, _, _, saver) = make_saver_bundle(Some(PathBuf::from("/stale.json")));

    // State without a kit_path (simulating a save with no kit loaded).
    let state = serde_json::json!({ "params": {}, "kit_path": serde_json::Value::Null });
    saver.load(&state);
    assert_eq!(*kit_path.lock(), None);
}

/// Per-pad close-mic choices and the global overhead setup round-trip
/// through the ExtraStateSaver JSON.
#[test]
fn extra_saver_roundtrips_mic_choices() {
    let (_, oh_arc, pad_arc, _, saver) = make_saver_bundle(None);
    // Inject user edits.
    *oh_arc.lock() = "24_OHsAB_KM184".to_string();
    {
        let mut guard = pad_arc.lock();
        guard[0]
            .close_setups
            .insert("KickIn".to_string(), "01_KickIn_e901".to_string());
        guard[0]
            .close_setups
            .insert("KickOut".to_string(), "05_KickOut_D01".to_string());
        guard[1]
            .close_setups
            .insert("SNTop".to_string(), "07_SNTop_e904".to_string());
    }

    let mut json = serde_json::json!({ "params": {} });
    for (k, v) in saver.save() {
        json.as_object_mut().unwrap().insert(k, v);
    }

    let (_, oh2, pad2, _, restored) = make_saver_bundle(None);
    restored.load(&json);
    assert_eq!(*oh2.lock(), "24_OHsAB_KM184");
    let guard = pad2.lock();
    assert_eq!(
        guard[0].close_setups.get("KickIn"),
        Some(&"01_KickIn_e901".to_string())
    );
    assert_eq!(
        guard[0].close_setups.get("KickOut"),
        Some(&"05_KickOut_D01".to_string())
    );
    assert_eq!(
        guard[1].close_setups.get("SNTop"),
        Some(&"07_SNTop_e904".to_string())
    );
}

/// The legacy `articulations` array is still written, so a build from
/// before ba todo #1325 can still read a project this one saves.
#[test]
fn extra_saver_still_writes_the_legacy_articulation_array() {
    let (_, _, _, params, saver) = make_saver_bundle(None);
    params.pads[0].articulation.set_value(ARTICULATION_ALT); // Kick -> ohne Teppich
    params.pads[9].articulation.set_value(ARTICULATION_ALT); // Tom High -> ohne Teppich

    let saved = saver.save();
    let arr = saved
        .get("articulations")
        .and_then(|v| v.as_array())
        .expect("legacy articulation array");
    assert_eq!(arr.len(), drum_map::NUM_PADS);
    assert_eq!(arr[0], serde_json::Value::Bool(true));
    assert_eq!(arr[9], serde_json::Value::Bool(true));
    assert_eq!(arr[1], serde_json::Value::Bool(false));
}

/// A project old enough to predate the `pad_N_articulation` parameter
/// still opens with its articulations: the legacy array is adopted for
/// pads the file has no parameter for.
#[test]
fn extra_saver_migrates_a_legacy_articulation_array() {
    let (_, _, _, params, saver) = make_saver_bundle(None);
    let mut legacy = vec![serde_json::Value::Bool(false); drum_map::NUM_PADS];
    legacy[1] = serde_json::Value::Bool(true);
    let state = serde_json::json!({ "params": {}, "articulations": legacy });

    saver.load(&state);
    assert_eq!(params.pads[1].articulation.value(), ARTICULATION_ALT);
    assert_eq!(params.pads[0].articulation.value(), ARTICULATION_PRIMARY);
}

/// When the file carries the parameter, the parameter wins — the params
/// are the source of truth and `load_params_from_json` has already
/// applied them by the time the saver runs.
#[test]
fn the_param_wins_over_the_legacy_articulation_array() {
    let (_, _, _, params, saver) = make_saver_bundle(None);
    // What `load_params_from_json` would have done first.
    params.pads[0].articulation.set_value(ARTICULATION_PRIMARY);
    let mut legacy = vec![serde_json::Value::Bool(false); drum_map::NUM_PADS];
    legacy[0] = serde_json::Value::Bool(true);
    let state = serde_json::json!({
        "params": { "pad_0_articulation": 0.0 },
        "articulations": legacy,
    });

    saver.load(&state);
    assert_eq!(
        params.pads[0].articulation.value(),
        ARTICULATION_PRIMARY,
        "the legacy array must not override a parameter the file carries"
    );
}

/// The whole sound of a drums preset (review M6): loading one that names
/// a kit while the plugin is running reloads that kit (not only the saved
/// path); a params-only preset keeps the current kit; the same kit again
/// does not reload; a mic choice change does.
#[test]
fn a_preset_load_reloads_the_kit_it_names_and_keeps_it_when_it_names_none() {
    let mut drums = ResonanceDrums::new();
    assert!(drums.initialize(48_000.0, 512));
    let stamps = drums.bridge.load_generation.clone();
    let generation = move || stamps.load(std::sync::atomic::Ordering::Acquire);
    let g0 = generation();

    let with_kit = serde_json::json!({
        "version": 1,
        "params": {},
        "kit_path": "/nonexistent/other-kit/drum_samples.json",
    });
    assert!(drums.load_state(&serde_json::to_vec(&with_kit).unwrap()));
    let g1 = generation();
    assert_eq!(g1, g0 + 1, "the named kit is loaded, not just remembered");

    let params_only = serde_json::json!({"version": 1, "params": {}});
    assert!(drums.load_state(&serde_json::to_vec(&params_only).unwrap()));
    assert_eq!(
        drums.bridge.kit_path.lock().clone(),
        Some(PathBuf::from("/nonexistent/other-kit/drum_samples.json")),
        "a preset without a kit keeps the kit"
    );
    let g2 = generation();
    assert_eq!(g2, g1, "and does not reload it");

    assert!(drums.load_state(&serde_json::to_vec(&with_kit).unwrap()));
    assert_eq!(
        generation(),
        g2,
        "the same kit again is not a reload"
    );

    let mut with_mics = with_kit.clone();
    with_mics["overhead_setup_key"] = serde_json::json!("room");
    assert!(drums.load_state(&serde_json::to_vec(&with_mics).unwrap()));
    assert_eq!(
        generation(),
        g2 + 1,
        "other mics are another sound"
    );
}

/// A full-state reload is deactivate → load → activate: the kit it names
/// is loaded once, by `initialize`, not also by the load in between
/// (verification item 7).
#[test]
fn a_full_state_reload_loads_the_kit_once() {
    let mut drums = ResonanceDrums::new();
    assert!(drums.initialize(48_000.0, 512));
    let stamps = drums.bridge.load_generation.clone();
    let generation = move || stamps.load(std::sync::atomic::Ordering::Acquire);
    let g0 = generation();
    drums.deactivate();
    let state = serde_json::json!({
        "version": 1,
        "params": {},
        "kit_path": "/nonexistent/reload-kit/drum_samples.json",
    });
    assert!(drums.load_state(&serde_json::to_vec(&state).unwrap()));
    assert_eq!(generation(), g0, "inactive: the load only records the kit");
    assert!(drums.initialize(48_000.0, 512));
    assert_eq!(generation(), g0 + 1, "one load, from initialize");
}
