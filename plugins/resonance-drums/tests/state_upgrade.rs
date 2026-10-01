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
