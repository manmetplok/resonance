use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use resonance_drums::kit_loader::{PadMicChoices, DEFAULT_OVERHEAD_SETUP};
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
    Arc<Mutex<[bool; drum_map::NUM_PADS]>>,
    DrumsExtraState,
);

/// Helper: build an empty saver storage bundle used by the saver tests.
fn make_saver_bundle(initial_path: Option<PathBuf>) -> SaverBundle {
    let kit_path = Arc::new(Mutex::new(initial_path));
    let overhead_setup_key = Arc::new(Mutex::new(DEFAULT_OVERHEAD_SETUP.to_string()));
    let pad_choices = Arc::new(Mutex::new(std::array::from_fn(|_| {
        PadMicChoices::default()
    })));
    let articulations = Arc::new(Mutex::new([false; drum_map::NUM_PADS]));
    let saver = DrumsExtraState {
        kit_path: kit_path.clone(),
        overhead_setup_key: overhead_setup_key.clone(),
        pad_choices: pad_choices.clone(),
        articulations: articulations.clone(),
    };
    (
        kit_path,
        overhead_setup_key,
        pad_choices,
        articulations,
        saver,
    )
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

/// Articulation toggles round-trip through the ExtraStateSaver JSON.
#[test]
fn extra_saver_roundtrips_articulations() {
    let (_, _, _, art_arc, saver) = make_saver_bundle(None);
    {
        let mut guard = art_arc.lock();
        guard[0] = true; // Kick -> ohne Teppich
        guard[9] = true; // Tom High -> ohne Teppich
    }

    let mut json = serde_json::json!({ "params": {} });
    for (k, v) in saver.save() {
        json.as_object_mut().unwrap().insert(k, v);
    }

    let (_, _, _, art2, restored) = make_saver_bundle(None);
    restored.load(&json);
    let guard = art2.lock();
    assert!(guard[0], "kick articulation should be true");
    assert!(guard[9], "tom high articulation should be true");
    assert!(!guard[1], "snare articulation should be false (default)");
}
