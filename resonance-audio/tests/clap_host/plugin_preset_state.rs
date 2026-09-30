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

    instance.set_param(threshold.id, loaded - 6.0);
    assert_eq!(last_modified(&mut instance), Some(true), "a host edit is an edit");
    instance.set_param(threshold.id, loaded);
    assert_eq!(last_modified(&mut instance), Some(false), "and back is not");

    assert!(instance.set_preset_ignored_params(&[threshold.id]));
    instance.set_param(threshold.id, loaded - 6.0);
    assert_eq!(
        last_modified(&mut instance),
        None,
        "an automated param moving is not an edit: nothing changed to report"
    );
    drop(instance);
    drop(bundle);
}
