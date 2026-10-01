//! The drums' state upgrade (`ResonanceDrums::STATE_UPGRADE`,
//! [`resonance_drums::upgrade_state`]) on every path a state reaches the
//! plugin by — through the real CLAP C ABI, in-process:
//!
//! - `state.load` while the plugin is **inactive** (its own `load_state`);
//! - `state.load` while it is **active**: its object is in the audio
//!   processor, and the bridge loads the shared atomics itself — the path
//!   that used to skip the v1 conversion;
//! - a preset (the bridge's `FOR_PRESET` load, and the preset bank's).
//!
//! And what reopening a v1 project does after the state: the host re-sends
//! the project's saved param values by id. Under v1's ids they name no
//! param any more, so the conversion stands.

use clack_extensions::params::PluginParams;
use clack_extensions::state::PluginState;
use clack_extensions::state_context::{PluginStateContext, StateContextType};
use clack_host::events::event_types::ParamValueEvent;
use clack_host::prelude::*;
use clack_host::utils::Cookie;
use clack_plugin::entry::SinglePluginEntry;
use resonance_drums::drum_map::NUM_PADS;
use resonance_drums::level::db_to_gain;
use resonance_drums::{library, ResonanceDrums};
use resonance_plugin::{stable_hash, ClapBridge, ResonancePlugin};
use serde_json::Value;

struct TestHostShared;

impl SharedHandler<'_> for TestHostShared {
    fn request_restart(&self) {}
    fn request_process(&self) {}
    fn request_callback(&self) {}
}

struct TestHost;

impl HostHandlers for TestHost {
    type Shared<'a> = TestHostShared;
    type MainThread<'a> = ();
    type AudioProcessor<'a> = ();
}

fn hosted() -> PluginInstance<TestHost> {
    library::isolate_for_tests();
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<ResonanceDrums>>>(
        c"resonance-drums-state-upgrade.clap",
    )
    .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();
    PluginInstance::<TestHost>::new(
        |_| TestHostShared,
        |_| (),
        &entry,
        c"com.resonance.drums",
        &host_info,
    )
    .expect("plugin instantiation")
}

fn audio_config() -> PluginAudioConfiguration {
    PluginAudioConfiguration {
        sample_rate: 48_000.0,
        min_frames_count: 32,
        max_frames_count: 8192,
    }
}

fn load(instance: &mut PluginInstance<TestHost>, state: &Value) -> bool {
    let ext = instance
        .plugin_shared_handle()
        .get_extension::<PluginState>()
        .expect("state extension");
    let bytes = serde_json::to_vec(state).unwrap();
    ext.load(&mut instance.plugin_handle(), &mut &bytes[..])
        .is_ok()
}

fn value(instance: &mut PluginInstance<TestHost>, id: &str) -> f64 {
    let ext = instance
        .plugin_shared_handle()
        .get_extension::<PluginParams>()
        .expect("params extension");
    ext.get_value(&mut instance.plugin_handle(), ClapId::new(stable_hash(id)))
        .unwrap_or_else(|| panic!("param `{id}` is unknown to the bridge"))
}

fn has_param(instance: &mut PluginInstance<TestHost>, id: &str) -> bool {
    let ext = instance
        .plugin_shared_handle()
        .get_extension::<PluginParams>()
        .expect("params extension");
    ext.get_value(&mut instance.plugin_handle(), ClapId::new(stable_hash(id)))
        .is_some()
}

/// The host re-sending a project's saved values by id, the plugin idle.
fn flush(instance: &mut PluginInstance<TestHost>, changes: &[(&str, f64)]) {
    let ext = instance
        .plugin_shared_handle()
        .get_extension::<PluginParams>()
        .expect("params extension");
    let mut input = EventBuffer::new();
    for (id, v) in changes {
        input.push(&ParamValueEvent::new(
            0,
            ClapId::new(stable_hash(id)),
            Pckn::match_all(),
            *v,
            Cookie::empty(),
        ));
    }
    let mut output = EventBuffer::new();
    let mut handle = instance
        .inactive_plugin_handle()
        .expect("the plugin must be inactive");
    ext.flush(&mut handle, &input.as_input(), &mut output.as_output());
}

/// A v1 project's drums state: linear levels (master 0.8, the kick 0.5),
/// a balance and an overhead blend on every pad, no `output_mode`.
fn v1_state() -> Value {
    let mut params = serde_json::Map::new();
    params.insert("master_volume".into(), 0.8.into());
    for i in 0..NUM_PADS {
        params.insert(format!("pad_{i}_volume"), 0.8.into());
        params.insert(format!("pad_{i}_balance"), 0.5.into());
        params.insert(format!("pad_{i}_oh_blend"), 1.0.into());
    }
    params.insert("pad_0_volume".into(), 0.5.into());
    serde_json::json!({ "version": 1, "params": params })
}

fn assert_converted(instance: &mut PluginInstance<TestHost>, path: &str) {
    let gain = |instance: &mut PluginInstance<TestHost>, id: &str| {
        db_to_gain(value(instance, id) as f32)
    };
    let master = gain(instance, "master_level");
    let kick = gain(instance, "pad_0_level");
    let snare = gain(instance, "pad_1_level");
    assert!((master - 0.8).abs() < 1e-4, "{path}: master plays {master}, v1 0.8");
    assert!((kick - 0.5).abs() < 1e-4, "{path}: the kick plays {kick}, v1 0.5");
    assert!((snare - 0.8).abs() < 1e-4, "{path}: the snare plays {snare}, v1 0.8");
}

#[test]
fn a_v1_state_converts_on_the_inactive_bridge_path() {
    let mut instance = hosted();
    assert!(load(&mut instance, &v1_state()));
    assert_converted(&mut instance, "inactive");
}

/// The path that used to skip the conversion: the bridge's own load
/// while the plugin is in the audio processor.
#[test]
fn a_v1_state_converts_on_the_active_bridge_path() {
    let mut instance = hosted();
    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");
    assert!(load(&mut instance, &v1_state()));
    assert_converted(&mut instance, "active");
    instance.deactivate(processor);
    assert_converted(&mut instance, "active, then deactivated");
}

/// A v1 project reopened: the state, then the project's saved values
/// re-sent by their v1 ids. They name no param, so they cannot land a
/// linear 0.8 on a dB param.
#[test]
fn a_v1_projects_stale_overrides_are_dropped_by_the_bridge() {
    let mut instance = hosted();
    assert!(load(&mut instance, &v1_state()));
    assert!(!has_param(&mut instance, "master_volume"));
    assert!(!has_param(&mut instance, "pad_0_volume"));
    flush(
        &mut instance,
        &[("master_volume", 0.8), ("pad_0_volume", 0.5), ("pad_1_volume", 0.8)],
    );
    assert_converted(&mut instance, "after the stale overrides");
}

#[test]
fn a_v1_preset_converts_on_the_bridges_preset_load() {
    let mut instance = hosted();
    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");
    let ext = instance
        .plugin_shared_handle()
        .get_extension::<PluginStateContext>()
        .expect("state-context extension");
    let bytes = serde_json::to_vec(&v1_state()).unwrap();
    ext.load(
        &mut instance.plugin_handle(),
        &mut &bytes[..],
        StateContextType::ForPreset,
    )
    .expect("preset load");
    assert_converted(&mut instance, "FOR_PRESET");
    instance.deactivate(processor);
}

#[test]
fn a_v1_preset_converts_through_the_preset_bank() {
    library::isolate_for_tests();
    let plugin = ResonanceDrums::new();
    let params: Vec<&dyn resonance_plugin::Param> =
        (0..plugin.param_count()).map(|i| plugin.param(i)).collect();
    let bank = resonance_plugin::presets::PresetBank::for_plugin::<ResonanceDrums>();
    assert!(bank.state_upgrade().is_some());
    assert!(resonance_plugin::presets::apply_with(
        &v1_state().to_string(),
        &params,
        bank.renames(),
        bank.state_upgrade(),
    ));
    let gain = |id: &str| {
        let p = params.iter().find(|p| p.id() == id).unwrap();
        db_to_gain(p.get_plain() as f32)
    };
    assert!((gain("master_level") - 0.8).abs() < 1e-4);
    assert!((gain("pad_0_level") - 0.5).abs() < 1e-4);
}

/// The upgrade is idempotent: a preset is upgraded, laid over the
/// current state, and the result upgraded again.
#[test]
fn the_upgrade_is_idempotent() {
    let mut once = v1_state();
    resonance_drums::upgrade_state(&mut once);
    let mut twice = once.clone();
    resonance_drums::upgrade_state(&mut twice);
    assert_eq!(once, twice);
}

// ---------------------------------------------------------------------------
// Output mode (E11, D5): Stereo for a fresh instance, Multi for a state
// saved before the param existed — it played multi-out.
// ---------------------------------------------------------------------------

const STEREO: f64 = resonance_drums::params::OUTPUT_MODE_STEREO as f64;
const MULTI: f64 = resonance_drums::params::OUTPUT_MODE_MULTI as f64;

#[test]
fn a_fresh_instance_is_stereo_and_stays_stereo_through_its_own_state() {
    let mut instance = hosted();
    assert_eq!(value(&mut instance, "output_mode"), STEREO);
    let ext = instance
        .plugin_shared_handle()
        .get_extension::<PluginState>()
        .expect("state extension");
    let mut saved = Vec::new();
    ext.save(&mut instance.plugin_handle(), &mut saved)
        .expect("save");
    let mut reopened = hosted();
    assert!(load(&mut reopened, &serde_json::from_slice(&saved).unwrap()));
    assert_eq!(value(&mut reopened, "output_mode"), STEREO);
}

#[test]
fn a_v1_state_loads_as_multi_on_both_bridge_paths() {
    let mut inactive = hosted();
    assert!(load(&mut inactive, &v1_state()));
    assert_eq!(value(&mut inactive, "output_mode"), MULTI);

    let mut active = hosted();
    let processor = active
        .activate(|_, _| (), audio_config())
        .expect("activation");
    assert!(load(&mut active, &v1_state()));
    assert_eq!(value(&mut active, "output_mode"), MULTI);
    active.deactivate(processor);
    assert_eq!(value(&mut active, "output_mode"), MULTI);
}

/// A v2 state from before K7 (dB levels under the new ids, no
/// `output_mode`) played multi-out too.
#[test]
fn a_pre_k7_state_without_an_output_mode_loads_as_multi() {
    let mut instance = hosted();
    let pre_k7 = serde_json::json!({ "version": 1, "params": { "pad_0_level": -3.0 } });
    assert!(load(&mut instance, &pre_k7));
    assert_eq!(value(&mut instance, "output_mode"), MULTI);
}

fn load_preset(instance: &mut PluginInstance<TestHost>, doc: &Value) {
    let ext = instance
        .plugin_shared_handle()
        .get_extension::<PluginStateContext>()
        .expect("state-context extension");
    let bytes = serde_json::to_vec(doc).unwrap();
    ext.load(
        &mut instance.plugin_handle(),
        &mut &bytes[..],
        StateContextType::ForPreset,
    )
    .expect("preset load");
}

/// Routing is how the instance is wired into its track, not the sound:
/// a preset neither carries nor recalls `output_mode` / `pad_N_output`.
#[test]
fn a_preset_never_reroutes_the_instance() {
    let mut instance = hosted();
    let state = serde_json::json!({ "version": 1, "params": {
        "output_mode": MULTI, "pad_0_output": 3.0,
    } });
    assert!(load(&mut instance, &state));
    load_preset(
        &mut instance,
        &serde_json::json!({ "version": 1, "params": {
            "output_mode": STEREO, "pad_0_output": 0.0, "pad_0_level": -6.0,
        } }),
    );
    assert_eq!(value(&mut instance, "pad_0_level"), -6.0, "the sound is recalled");
    assert_eq!(value(&mut instance, "output_mode"), MULTI);
    assert_eq!(value(&mut instance, "pad_0_output"), 3.0);

    // A v1 preset (no mode: the upgrade gives it Multi) on a Stereo
    // instance leaves it Stereo.
    let mut stereo = hosted();
    load_preset(&mut stereo, &v1_state());
    assert_converted(&mut stereo, "v1 preset");
    assert_eq!(value(&mut stereo, "output_mode"), STEREO);

    // And a preset saved from an instance does not carry them.
    let plugin = ResonanceDrums::new();
    let params: Vec<&dyn resonance_plugin::Param> =
        (0..plugin.param_count()).map(|i| plugin.param(i)).collect();
    let doc = resonance_plugin::presets::preset_params_json(&params);
    let saved = doc["params"].as_object().unwrap();
    assert!(!saved.contains_key("output_mode"));
    assert!((0..NUM_PADS).all(|i| !saved.contains_key(&format!("pad_{i}_output"))));
    assert!(saved.contains_key("pad_0_level"));
}

#[test]
fn a_state_that_names_its_mode_keeps_it() {
    for mode in [STEREO, MULTI] {
        let mut instance = hosted();
        let state = serde_json::json!({ "version": 1, "params": {
            "pad_0_level": -3.0, "output_mode": mode,
        } });
        assert!(load(&mut instance, &state));
        assert_eq!(value(&mut instance, "output_mode"), mode);
    }
}

// ---------------------------------------------------------------------------
// Polyphony (E15): 64 was the maximum before a hit could take 8 voices.
// ---------------------------------------------------------------------------

/// A state from before E15 (no bank param, no `mic_banks`) with
/// `polyphony` at 64 — that build's "every voice" — loads at today's 128;
/// one with E15's keys, or below 64, loads as saved. Idempotent.
#[test]
fn a_pre_e15_polyphony_at_its_maximum_loads_at_todays() {
    let state = |polyphony: f64, extra: &[(&str, Value)]| {
        let mut params = serde_json::Map::new();
        params.insert("master_level".into(), 0.0.into());
        params.insert("output_mode".into(), STEREO.into());
        params.insert("polyphony".into(), polyphony.into());
        let mut doc = serde_json::json!({ "version": 2 });
        for (key, value) in extra {
            if key.starts_with("mic_banks") {
                doc[*key] = value.clone();
            } else {
                params.insert(key.to_string(), value.clone());
            }
        }
        doc["params"] = Value::Object(params);
        doc
    };

    let mut instance = hosted();
    assert!(load(&mut instance, &state(64.0, &[])));
    assert_eq!(value(&mut instance, "polyphony"), 128.0, "the old maximum");
    assert!(load(&mut instance, &state(32.0, &[])));
    assert_eq!(value(&mut instance, "polyphony"), 32.0, "a chosen limit");
    assert!(load(&mut instance, &state(64.0, &[("bleed_on", 0.0.into())])));
    assert_eq!(value(&mut instance, "polyphony"), 64.0, "an E15 state's 64");
    let banks = serde_json::json!({"overheads": ["", ""], "room": ""});
    assert!(load(&mut instance, &state(64.0, &[("mic_banks", banks)])));
    assert_eq!(value(&mut instance, "polyphony"), 64.0, "an E15 state's 64");

    let mut once = state(64.0, &[]);
    resonance_drums::upgrade_state(&mut once);
    assert_eq!(once["params"]["polyphony"], 128);
    let mut twice = once.clone();
    resonance_drums::upgrade_state(&mut twice);
    assert_eq!(once, twice);
}

/// The instance's saved state.
fn saved(instance: &mut PluginInstance<TestHost>) -> Value {
    let ext = instance
        .plugin_shared_handle()
        .get_extension::<PluginState>()
        .expect("state extension");
    let mut bytes = Vec::new();
    ext.save(&mut instance.plugin_handle(), &mut bytes)
        .expect("save");
    serde_json::from_slice(&bytes).unwrap()
}

/// A preset from before E15 names the mic choices but no `mic_banks`: it
/// predates the banks, so recalling it over an instance with banks set
/// empties them, as reopening such a project does. A preset with no mic
/// choices at all (params only) leaves them alone.
#[test]
fn a_pre_e15_preset_recalls_no_extra_banks() {
    let banks = serde_json::json!({"overheads": ["25_OHsXY", ""], "room": "31_RoomFar"});
    let mut instance = hosted();
    assert!(load(
        &mut instance,
        &serde_json::json!({ "version": 2, "params": {}, "mic_banks": banks.clone() })
    ));
    assert_eq!(saved(&mut instance)["mic_banks"], banks);

    load_preset(
        &mut instance,
        &serde_json::json!({ "version": 2, "params": { "pad_0_level": -1.0 } }),
    );
    assert_eq!(saved(&mut instance)["mic_banks"], banks, "params only: kept");

    load_preset(
        &mut instance,
        &serde_json::json!({ "version": 2, "params": { "pad_0_level": -2.0 },
            "overhead_setup_key": "23_OHsAB_e914" }),
    );
    assert_eq!(
        saved(&mut instance)["mic_banks"],
        serde_json::json!({"overheads": ["", ""], "room": ""}),
        "a pre-E15 preset meant no extra banks"
    );
}
