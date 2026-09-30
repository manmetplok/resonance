//! The preset form of a first-party plugin's state, through the real CLAP
//! ABI (plugin-preset-library.md §6.7, §9.2; slices P2/P5):
//! `clap.state-context` `save(FOR_PRESET)` writes the params that belong
//! in a preset plus the sound-bearing extra state; `load(FOR_PRESET)` lays
//! a preset over the current state; `clap.preset-load` loads a factory
//! preset by id or a preset file by path.
//!
//! Driven against built plugins (see `plugin_binaries`).

use resonance_audio::test_support::ClapBundle;
use resonance_audio::types::PluginPresetLocation;

use crate::plugin_binaries::plugin_binary;

fn state_of(instance: &resonance_audio::test_support::ClapInstance) -> serde_json::Value {
    serde_json::from_slice(&instance.save_state().expect("save_state")).unwrap()
}

/// The IR's preset form carries `ir_path` (the sound) and leaves out
/// `file_select` (an index into this machine's directory listing); a
/// preset loaded over it replaces exactly those. A params-only preset (no
/// `ir_path`: a legacy file) keeps the current IR, which is what the IR's
/// state loader does with a document that lacks the key.
#[test]
fn the_ir_preset_form_carries_the_impulse_not_the_index() {
    let Some(path) = plugin_binary("resonance-ir") else {
        return;
    };
    let bundle = ClapBundle::load(&path).expect("the IR bundle should load");
    let id = bundle.descriptors()[0].id.clone();
    let mut instance = bundle.create_instance(&id, 48_000).expect("create_instance");
    assert!(instance.has_preset_state(), "first-party plugins implement state-context");
    assert!(instance.has_preset_load(), "and preset-load");
    assert!(instance.is_first_party(), "and the preset-session extension: provenance");

    let (form, preset_form) = instance.save_preset_state().expect("preset state");
    assert!(preset_form);
    let doc: serde_json::Value = serde_json::from_slice(&form).unwrap();
    let params = doc["params"].as_object().expect("params");
    assert!(!params.contains_key("file_select"), "excluded from presets: {doc}");
    assert!(params.contains_key("dry_wet"), "{doc}");
    assert!(doc.get("ir_path").is_some(), "the IR is the sound: {doc}");

    // Lay a preset over the state: a new mix and a (nonexistent, so never
    // actually loaded) IR path, with an identity.
    let preset = serde_json::json!({
        "version": 1,
        "params": {"dry_wet": 0.25, "file_select": 7.0},
        "ir_path": "/nonexistent/cab.wav",
        "preset": {"id": "p-1", "name": "Cab", "source": "user", "modified": false},
    });
    assert!(instance.load_preset_state(&serde_json::to_vec(&preset).unwrap()));
    let state = state_of(&instance);
    assert!((state["params"]["dry_wet"].as_f64().unwrap() - 0.25).abs() < 1e-6);
    assert_eq!(state["params"]["file_select"].as_f64(), Some(0.0), "excluded: kept");
    assert_eq!(state["ir_path"], "/nonexistent/cab.wav");
    assert_eq!(state["preset"]["id"], "p-1", "the plugin's bar names the preset");

    let bare = serde_json::json!({"version": 1, "params": {"dry_wet": 0.5}});
    assert!(instance.load_preset_state(&serde_json::to_vec(&bare).unwrap()));
    let state = state_of(&instance);
    assert_eq!(state["ir_path"], "/nonexistent/cab.wav", "{state}");
    assert!((state["params"]["dry_wet"].as_f64().unwrap() - 0.5).abs() < 1e-6);
    assert!(
        state.get("preset").is_none(),
        "a preset naming no identity drops the old one: {state}"
    );
    drop(instance);
    drop(bundle);
}

/// `clap.preset-load`: a factory preset by id (`PLUGIN` location) and a
/// preset file by path (`FILE`) both load the whole preset and name it.
#[test]
fn preset_load_takes_a_factory_id_or_a_file() {
    let Some(path) = plugin_binary("resonance-gate") else {
        return;
    };
    let bundle = ClapBundle::load(&path).expect("the gate bundle should load");
    let id = bundle.descriptors()[0].id.clone();
    let factory = bundle.factory_presets().to_vec();
    let snare = factory
        .iter()
        .find(|p| p.id == "drums-snare-gate")
        .expect("the gate ships Drums — Snare Gate")
        .clone();
    let mut instance = bundle.create_instance(&id, 48_000).expect("create_instance");

    assert!(instance.load_preset_from_location(&PluginPresetLocation::Plugin, Some(&snare.id)));
    let state = state_of(&instance);
    let want: serde_json::Value = serde_json::from_str(&snare.json).unwrap();
    assert_eq!(state["params"]["threshold"], want["params"]["threshold"]);
    assert_eq!(state["preset"]["id"], "drums-snare-gate");
    assert_eq!(state["preset"]["source"], "factory");

    assert!(
        !instance.load_preset_from_location(&PluginPresetLocation::Plugin, Some("no-such-preset")),
        "an unknown id is refused"
    );

    // A user preset file.
    let dir = std::env::temp_dir().join(format!("resonance-preset-load-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("mine.json");
    std::fs::write(
        &file,
        serde_json::json!({
            "format": "resonance.preset", "format_version": 1,
            "id": "3f0c9a4e-7a51-4d7e-9b1e-5b2a8f1c0d42",
            "plugin": {"id": id},
            "meta": {"name": "Mine"},
            "state": {"encoding": "resonance-json", "doc": {"version": 1, "params": {"threshold": -12.5}}},
        })
        .to_string(),
    )
    .unwrap();
    assert!(instance.load_preset_from_location(&PluginPresetLocation::File(file.clone()), None));
    let state = state_of(&instance);
    assert_eq!(state["params"]["threshold"].as_f64(), Some(-12.5));
    assert_eq!(state["preset"]["name"], "Mine");
    assert_eq!(state["preset"]["source"], "user");
    let _ = std::fs::remove_dir_all(&dir);
    drop(instance);
    drop(bundle);
}

/// The identity reaches the host (`com.resonance.preset-session`, slice
/// P5): a factory load is reported with the identity and CLAP's own
/// `loaded()`; a host param change flips `modified` (the plugin compares on
/// its main thread, editor or not), moving it back clears it, and a param
/// the host says it automates is left out of the comparison.
#[test]
fn the_plugin_reports_its_preset_and_whether_it_was_edited() {
    use resonance_audio::test_support::PresetHostReport as R;
    let Some(path) = plugin_binary("resonance-gate") else {
        return;
    };
    let bundle = ClapBundle::load(&path).expect("the gate bundle should load");
    let id = bundle.descriptors()[0].id.clone();
    let mut instance = bundle.create_instance(&id, 48_000).expect("create_instance");
    let _ = instance.take_preset_reports();

    assert!(instance.load_preset_from_location(
        &PluginPresetLocation::Plugin,
        Some("drums-snare-gate")
    ));
    let reports = instance.take_preset_reports();
    let identity = reports.iter().find_map(|r| match r {
        R::Identity(Some(i)) => Some(i.clone()),
        _ => None,
    });
    let identity = identity.unwrap_or_else(|| panic!("an identity report: {reports:?}"));
    assert_eq!(identity.id, "drums-snare-gate");
    assert_eq!(identity.source, "factory");
    assert!(!identity.modified);
    assert!(
        reports.iter().any(|r| matches!(r,
            R::Loaded { location: PluginPresetLocation::Plugin, load_key: Some(k) }
                if k == "drums-snare-gate")),
        "CLAP's loaded() for other hosts too: {reports:?}"
    );

    let threshold = instance
        .query_params()
        .into_iter()
        .find(|p| p.name.eq_ignore_ascii_case("threshold"))
        .expect("the gate has a Threshold");
    let loaded = threshold.current_value;
    let mut last_modified = |instance: &mut resonance_audio::test_support::ClapInstance| {
        assert!(instance.flush_pending_params());
        instance.run_requested_callback();
        instance.take_preset_reports().into_iter().rev().find_map(|r| match r {
            R::Identity(Some(i)) => Some(i.modified),
            _ => None,
        })
    };

    let interval = std::time::Duration::from_millis(120);
    instance.set_param(threshold.id, loaded - 6.0);
    assert_eq!(last_modified(&mut instance), Some(true), "a host edit is an edit");

    // The comparison is throttled: moving back within the interval defers
    // it (still armed, a callback still requested) rather than dropping it.
    instance.set_param(threshold.id, loaded);
    assert_eq!(last_modified(&mut instance), None, "deferred");
    assert!(instance.has_requested_callback(), "and re-armed");
    std::thread::sleep(interval);
    instance.run_requested_callback();
    let back = instance.take_preset_reports().into_iter().rev().find_map(|r| match r {
        R::Identity(Some(i)) => Some(i.modified),
        _ => None,
    });
    assert_eq!(back, Some(false), "and back is not an edit");

    // An automated param's events do not even ask for a comparison: a
    // playing lane on a large-state plugin must cost nothing.
    assert!(instance.set_preset_ignored_params(&[threshold.id]));
    std::thread::sleep(interval);
    instance.run_requested_callback();
    let _ = instance.take_preset_reports();
    instance.set_param(threshold.id, loaded - 6.0);
    assert!(instance.flush_pending_params());
    assert!(
        !instance.has_requested_callback(),
        "an ignored param triggers no compare"
    );
    let ratio = instance
        .query_params()
        .into_iter()
        .find(|p| p.id != threshold.id && p.max_value > p.min_value)
        .expect("another param");
    instance.set_param(ratio.id, ratio.max_value);
    assert!(instance.flush_pending_params());
    assert!(instance.has_requested_callback(), "any other param does");
    drop(instance);
    drop(bundle);
}

// ---------------------------------------------------------------------------
// The engine's handlers (review: engine-handler tests)
// ---------------------------------------------------------------------------

mod handlers {
    use resonance_audio::test_support::EngineHandlerHarness;
    use resonance_audio::types::{AudioCommand, AudioEvent, PluginPresetLocation};
    use resonance_audio::{Track, TrackId};

    use crate::plugin_binaries::plugin_binary;

    const TRACK: TrackId = 1;
    const GATE: u64 = 7;

    fn gate() -> Option<EngineHandlerHarness> {
        let path = plugin_binary("resonance-gate")?.to_string_lossy().into_owned();
        let mut h = EngineHandlerHarness::new();
        h.push_track(Track::new(TRACK, "T1".to_string()));
        h.add_plugin(TRACK, path, "com.resonance.gate".to_string(), GATE);
        let _ = h.drain_events();
        Some(h)
    }

    fn threshold(events: &[AudioEvent]) -> Option<f64> {
        events.iter().rev().find_map(|e| match e {
            AudioEvent::PluginParamsRefreshed { instance_id: GATE, params } => params
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case("threshold"))
                .map(|p| p.current_value),
            _ => None,
        })
    }

    fn errors(events: &[AudioEvent]) -> usize {
        events.iter().filter(|e| matches!(e, AudioEvent::Error(_))).count()
    }

    /// `LoadPluginPresetState` with a capture: the full state before the
    /// load comes back under the token, then the mirror refresh with the
    /// loaded values — now, and again at the next host-request poll.
    #[test]
    fn a_preset_state_load_captures_first_and_refreshes_the_mirror() {
        let Some(mut h) = gate() else { return };
        let preset = serde_json::json!({"version": 1, "params": {"threshold": -21.0}});
        h.dispatch(AudioCommand::LoadPluginPresetState {
            instance_id: GATE,
            data: serde_json::to_vec(&preset).unwrap(),
            capture: Some(42),
        });
        let events = h.drain_events();
        let captured = events.iter().find_map(|e| match e {
            AudioEvent::PluginStateCaptured { instance_id: GATE, token: 42, data } => Some(data),
            _ => None,
        });
        let before: serde_json::Value =
            serde_json::from_slice(captured.expect("a capture")).unwrap();
        assert_ne!(before["params"]["threshold"].as_f64(), Some(-21.0), "captured before");
        assert_eq!(threshold(&events), Some(-21.0));
        h.poll_plugin_host_requests();
        assert_eq!(threshold(&h.drain_events()), Some(-21.0), "and after the next block");
    }

    /// `LoadPluginPresetFromLocation`: a factory id loads and refreshes;
    /// an unknown one is exactly one error.
    #[test]
    fn a_location_load_refreshes_and_a_failure_is_one_error() {
        let Some(mut h) = gate() else { return };
        h.dispatch(AudioCommand::LoadPluginPresetFromLocation {
            instance_id: GATE,
            location: PluginPresetLocation::Plugin,
            load_key: Some("drums-snare-gate".into()),
            capture: None,
        });
        let events = h.drain_events();
        assert!(threshold(&events).is_some(), "{events:?}");
        assert_eq!(errors(&events), 0);
        h.poll_plugin_host_requests();
        let polled = h.drain_events();
        assert!(
            polled.iter().any(|e| matches!(
                e,
                AudioEvent::PluginPresetIdentity { instance_id: GATE, identity: Some(i) }
                    if i.id == "drums-snare-gate"
            )),
            "the plugin reports its identity from on_main_thread: {polled:?}"
        );

        h.dispatch(AudioCommand::LoadPluginPresetFromLocation {
            instance_id: GATE,
            location: PluginPresetLocation::Plugin,
            load_key: Some("no-such-preset".into()),
            capture: None,
        });
        let mut events = h.drain_events();
        h.poll_plugin_host_requests();
        events.extend(h.drain_events());
        assert_eq!(errors(&events), 1, "one error, not two: {events:?}");
    }
}
