//! Host-side tests for the CLAP bridge's parameter and state plumbing
//! (`src/clap_bridge/params.rs` and `src/clap_bridge/state.rs`).
//!
//! These drive the bridge through the real CLAP C ABI (clack-host,
//! in-process) rather than poking at internals, because the interesting
//! behaviour only exists across that boundary: while the plugin is
//! **active** its object lives in the audio processor, so `params.get_value`
//! and `state.save`/`state.load` on the main thread must serve everything
//! from the shared atomics instead. That second code path is the one every
//! project save/load hits when the transport is running, and it has to
//! agree with the inactive path value-for-value — otherwise reopening a
//! project would restore different settings depending on whether the
//! plugin happened to be active.

use std::ffi::CStr;
use std::sync::{Arc, Mutex, OnceLock};

use clack_extensions::params::{ParamInfoBuffer, ParamInfoFlags, PluginParams};
use clack_extensions::state::PluginState;
use clack_host::events::event_types::ParamValueEvent;
use clack_host::prelude::*;
use clack_host::utils::Cookie;
use clack_plugin::entry::SinglePluginEntry;

use resonance_plugin::{
    stable_hash, BoolParam, ClapBridge, EventIterator, ExtraStateSaver, FloatParam, FloatRange,
    IntParam, IntRange, OutputBuffer, Param, ResonancePlugin, TempoInfo,
};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Test plugin: a representative param set (continuous, stepped, boolean,
// hidden) so the bridge's visibility filter and stepped flag are exercised.
// ---------------------------------------------------------------------------

struct BridgePlugin {
    mix: FloatParam,
    taps: IntParam,
    bypass: BoolParam,
    internal: FloatParam,
}

const MIX_DEFAULT: f64 = 0.5;
const TAPS_DEFAULT: f64 = 3.0;
const INTERNAL_DEFAULT: f64 = -6.0;

impl BridgePlugin {
    fn params(&self) -> [&dyn Param; 4] {
        [&self.mix, &self.taps, &self.bypass, &self.internal]
    }
}

impl ResonancePlugin for BridgePlugin {
    const CLAP_ID: &'static str = "test.bridge-params";
    const NAME: &'static str = "BridgeParams";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static str] = &[];
    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            mix: FloatParam::new(
                "mix",
                "Mix",
                MIX_DEFAULT as f32,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_unit("%"),
            taps: IntParam::new(
                "taps",
                "Taps",
                TAPS_DEFAULT as i32,
                IntRange::Linear { min: 1, max: 8 },
            ),
            bypass: BoolParam::new("bypass", "Bypass", false),
            internal: FloatParam::new(
                "internal",
                "Internal",
                INTERNAL_DEFAULT as f32,
                FloatRange::Linear {
                    min: -24.0,
                    max: 24.0,
                },
            )
            .hidden(),
        }
    }
    fn param_count(&self) -> usize {
        4
    }
    fn param(&self, index: usize) -> &dyn Param {
        self.params()[index]
    }
    fn initialize(&mut self, _sample_rate: f32, _max_buffer_size: u32) -> bool {
        true
    }
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _outputs: &mut [OutputBuffer<'_>],
        _frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
    }
}

// ---------------------------------------------------------------------------
// Test plugin #2: carries extra (non-param) state through a process-wide
// shared saver so the test can observe what the bridge saved and loaded.
// Only ONE test uses this type, so there is no cross-test interference.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct GlobalExtra {
    path: Mutex<String>,
    loaded: Mutex<Option<Value>>,
}

impl ExtraStateSaver for GlobalExtra {
    fn save(&self) -> serde_json::Map<String, Value> {
        let mut map = serde_json::Map::new();
        map.insert(
            "ir_path".to_string(),
            json!(self.path.lock().unwrap().clone()),
        );
        map
    }
    fn load(&self, state: &Value) {
        *self.loaded.lock().unwrap() = Some(state.clone());
        if let Some(p) = state.get("ir_path").and_then(|v| v.as_str()) {
            *self.path.lock().unwrap() = p.to_string();
        }
    }
}

static EXTRA: OnceLock<Arc<GlobalExtra>> = OnceLock::new();

fn extra() -> Arc<GlobalExtra> {
    EXTRA
        .get_or_init(|| Arc::new(GlobalExtra::default()))
        .clone()
}

struct ExtraStatePlugin {
    gain: FloatParam,
}

impl ResonancePlugin for ExtraStatePlugin {
    const CLAP_ID: &'static str = "test.bridge-extra";
    const NAME: &'static str = "BridgeExtra";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static str] = &[];
    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            gain: FloatParam::new(
                "gain",
                "Gain",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
        }
    }
    fn param_count(&self) -> usize {
        1
    }
    fn param(&self, _index: usize) -> &dyn Param {
        &self.gain
    }
    fn initialize(&mut self, _sample_rate: f32, _max_buffer_size: u32) -> bool {
        true
    }
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _outputs: &mut [OutputBuffer<'_>],
        _frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
    }
    fn extra_state_saver(&self) -> Option<Arc<dyn ExtraStateSaver>> {
        Some(extra() as Arc<dyn ExtraStateSaver>)
    }
}

// ---------------------------------------------------------------------------
// Minimal clack host
// ---------------------------------------------------------------------------

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

fn instantiate<P: ResonancePlugin>(bundle: &CStr, plugin_id: &CStr) -> PluginInstance<TestHost> {
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<P>>>(bundle)
        .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();

    PluginInstance::<TestHost>::new(|_| TestHostShared, |_| (), &entry, plugin_id, &host_info)
        .expect("plugin instantiation")
}

fn bridge_instance() -> PluginInstance<TestHost> {
    instantiate::<BridgePlugin>(c"resonance-test-bridge-params.clap", c"test.bridge-params")
}

fn audio_config() -> PluginAudioConfiguration {
    PluginAudioConfiguration {
        sample_rate: 48_000.0,
        min_frames_count: 32,
        max_frames_count: 8192,
    }
}

// ---------------------------------------------------------------------------
// Host-side helpers
// ---------------------------------------------------------------------------

fn params_ext(instance: &PluginInstance<TestHost>) -> PluginParams {
    instance
        .plugin_shared_handle()
        .get_extension::<PluginParams>()
        .expect("the bridge must expose the params extension")
}

fn state_ext(instance: &PluginInstance<TestHost>) -> PluginState {
    instance
        .plugin_shared_handle()
        .get_extension::<PluginState>()
        .expect("the bridge must expose the state extension")
}

fn clap_id(id: &str) -> ClapId {
    ClapId::new(stable_hash(id))
}

fn get_value(instance: &mut PluginInstance<TestHost>, id: &str) -> f64 {
    let ext = params_ext(instance);
    ext.get_value(&mut instance.plugin_handle(), clap_id(id))
        .unwrap_or_else(|| panic!("param `{id}` is unknown to the bridge"))
}

/// Deliver a param change the way a host does when the plugin is idle.
fn flush_inactive(instance: &mut PluginInstance<TestHost>, changes: &[(&str, f64)]) {
    let ext = params_ext(instance);
    let mut input = EventBuffer::new();
    for (id, value) in changes {
        input.push(&ParamValueEvent::new(
            0,
            clap_id(id),
            Pckn::match_all(),
            *value,
            Cookie::empty(),
        ));
    }
    let mut output = EventBuffer::new();
    let mut handle = instance
        .inactive_plugin_handle()
        .expect("the plugin must be inactive");
    ext.flush(&mut handle, &input.as_input(), &mut output.as_output());
}

fn save_state(instance: &mut PluginInstance<TestHost>) -> Vec<u8> {
    let ext = state_ext(instance);
    let mut bytes = Vec::new();
    ext.save(&mut instance.plugin_handle(), &mut bytes)
        .expect("state save");
    bytes
}

fn load_state(instance: &mut PluginInstance<TestHost>, bytes: &[u8]) -> bool {
    let ext = state_ext(instance);
    ext.load(&mut instance.plugin_handle(), &mut &bytes[..])
        .is_ok()
}

fn state_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("the bridge must save valid JSON")
}

// ---------------------------------------------------------------------------
// params: enumeration
// ---------------------------------------------------------------------------

#[test]
fn hidden_params_are_not_exposed_to_the_host() {
    let mut instance = bridge_instance();
    let ext = params_ext(&instance);

    assert_eq!(
        ext.count(&mut instance.plugin_handle()),
        3,
        "the hidden param must not be enumerated"
    );

    let mut buffer = ParamInfoBuffer::new();
    let names: Vec<String> = (0..3)
        .map(|i| {
            let info = ext
                .get_info(&mut instance.plugin_handle(), i, &mut buffer)
                .expect("every index below count() must resolve");
            String::from_utf8(info.name.to_vec()).unwrap()
        })
        .collect();
    assert_eq!(names, ["Mix", "Taps", "Bypass"]);

    // …but it is still reachable by id, so saved state keeps working.
    assert_eq!(get_value(&mut instance, "internal"), INTERNAL_DEFAULT);
}

#[test]
fn param_info_carries_the_declared_range_and_flags() {
    let mut instance = bridge_instance();
    let ext = params_ext(&instance);
    let mut buffer = ParamInfoBuffer::new();

    let mix = ext
        .get_info(&mut instance.plugin_handle(), 0, &mut buffer)
        .unwrap();
    assert_eq!(mix.id, clap_id("mix"));
    assert_eq!(mix.min_value, 0.0);
    assert_eq!(mix.max_value, 1.0);
    assert_eq!(mix.default_value, MIX_DEFAULT);
    assert!(mix.flags.contains(ParamInfoFlags::IS_AUTOMATABLE));
    assert!(
        !mix.flags.contains(ParamInfoFlags::IS_STEPPED),
        "a float param must not be marked stepped"
    );

    let taps = ext
        .get_info(&mut instance.plugin_handle(), 1, &mut buffer)
        .unwrap();
    assert_eq!(taps.min_value, 1.0);
    assert_eq!(taps.max_value, 8.0);
    assert_eq!(taps.default_value, TAPS_DEFAULT);
    assert!(taps.flags.contains(ParamInfoFlags::IS_STEPPED));

    let bypass = ext
        .get_info(&mut instance.plugin_handle(), 2, &mut buffer)
        .unwrap();
    assert_eq!(bypass.min_value, 0.0);
    assert_eq!(bypass.max_value, 1.0);
    assert!(bypass.flags.contains(ParamInfoFlags::IS_STEPPED));
}

#[test]
fn an_out_of_bounds_param_index_is_refused() {
    let mut instance = bridge_instance();
    let ext = params_ext(&instance);
    let mut buffer = ParamInfoBuffer::new();

    assert!(ext
        .get_info(&mut instance.plugin_handle(), 3, &mut buffer)
        .is_none());
    assert!(ext
        .get_info(&mut instance.plugin_handle(), 9999, &mut buffer)
        .is_none());
}

#[test]
fn an_unknown_param_id_has_no_value() {
    let mut instance = bridge_instance();
    let ext = params_ext(&instance);

    assert!(ext
        .get_value(&mut instance.plugin_handle(), clap_id("no_such_param"))
        .is_none());
}

#[test]
fn get_value_starts_at_the_plugin_defaults() {
    let mut instance = bridge_instance();

    assert_eq!(get_value(&mut instance, "mix"), MIX_DEFAULT);
    assert_eq!(get_value(&mut instance, "taps"), TAPS_DEFAULT);
    assert_eq!(get_value(&mut instance, "bypass"), 0.0);
    assert_eq!(get_value(&mut instance, "internal"), INTERNAL_DEFAULT);
}

// ---------------------------------------------------------------------------
// params: text conversion
// ---------------------------------------------------------------------------

#[test]
fn value_to_text_uses_the_params_own_formatter() {
    let mut instance = bridge_instance();
    let ext = params_ext(&instance);
    let mut buffer = [0u8; 128];

    let text = ext
        .value_to_text(
            &mut instance.plugin_handle(),
            clap_id("mix"),
            0.25,
            &mut buffer,
        )
        .expect("value_to_text");
    assert_eq!(std::str::from_utf8(text).unwrap(), "0.25%");

    let text = ext
        .value_to_text(
            &mut instance.plugin_handle(),
            clap_id("taps"),
            4.0,
            &mut buffer,
        )
        .expect("value_to_text");
    assert_eq!(std::str::from_utf8(text).unwrap(), "4");

    let text = ext
        .value_to_text(
            &mut instance.plugin_handle(),
            clap_id("bypass"),
            1.0,
            &mut buffer,
        )
        .expect("value_to_text");
    assert_eq!(std::str::from_utf8(text).unwrap(), "On");
}

#[test]
fn text_to_value_parses_what_value_to_text_produced() {
    let mut instance = bridge_instance();
    let ext = params_ext(&instance);

    assert_eq!(
        ext.text_to_value(&mut instance.plugin_handle(), clap_id("taps"), c"6"),
        Some(6.0)
    );
    assert_eq!(
        ext.text_to_value(&mut instance.plugin_handle(), clap_id("bypass"), c"Off"),
        Some(0.0)
    );
    assert_eq!(
        ext.text_to_value(&mut instance.plugin_handle(), clap_id("mix"), c"0.25%"),
        Some(0.25)
    );
    // Unparseable text is refused rather than resolving to 0.
    assert_eq!(
        ext.text_to_value(&mut instance.plugin_handle(), clap_id("taps"), c"lots"),
        None
    );
    // So is an unknown param.
    assert_eq!(
        ext.text_to_value(&mut instance.plugin_handle(), clap_id("nope"), c"1"),
        None
    );
}

// ---------------------------------------------------------------------------
// params: flush
// ---------------------------------------------------------------------------

#[test]
fn flushing_a_param_change_updates_both_the_plugin_and_the_shared_value() {
    let mut instance = bridge_instance();

    flush_inactive(
        &mut instance,
        &[("mix", 0.25), ("taps", 7.0), ("bypass", 1.0)],
    );

    assert_eq!(get_value(&mut instance, "mix"), 0.25);
    assert_eq!(get_value(&mut instance, "taps"), 7.0);
    assert_eq!(get_value(&mut instance, "bypass"), 1.0);

    // The change also reaches the plugin's own params, which is what a
    // subsequent state save serializes.
    let state = state_json(&save_state(&mut instance));
    assert_eq!(state["params"]["mix"], json!(0.25));
    assert_eq!(state["params"]["taps"], json!(7.0));
    assert_eq!(state["params"]["bypass"], json!(1.0));
}

#[test]
fn flushing_an_unknown_param_id_is_ignored() {
    let mut instance = bridge_instance();

    flush_inactive(&mut instance, &[("no_such_param", 0.9), ("mix", 0.75)]);

    // The known change still applied; the unknown one did nothing.
    assert_eq!(get_value(&mut instance, "mix"), 0.75);
}

// ---------------------------------------------------------------------------
// state: the save format
// ---------------------------------------------------------------------------

#[test]
fn saved_state_has_the_documented_shape_and_covers_hidden_params() {
    let mut instance = bridge_instance();
    let state = state_json(&save_state(&mut instance));

    let params = state["params"]
        .as_object()
        .expect("state must carry a `params` object");
    assert_eq!(params.len(), 4, "hidden params are persisted too");
    assert_eq!(params["mix"], json!(MIX_DEFAULT));
    assert_eq!(params["taps"], json!(TAPS_DEFAULT));
    assert_eq!(params["bypass"], json!(0.0));
    assert_eq!(params["internal"], json!(INTERNAL_DEFAULT));
}

#[test]
fn state_round_trips_while_the_plugin_is_inactive() {
    let mut source = bridge_instance();
    flush_inactive(
        &mut source,
        &[
            ("mix", 0.125),
            ("taps", 6.0),
            ("bypass", 1.0),
            ("internal", 12.0),
        ],
    );
    let bytes = save_state(&mut source);

    let mut target = bridge_instance();
    assert!(load_state(&mut target, &bytes));

    assert_eq!(get_value(&mut target, "mix"), 0.125);
    assert_eq!(get_value(&mut target, "taps"), 6.0);
    assert_eq!(get_value(&mut target, "bypass"), 1.0);
    assert_eq!(get_value(&mut target, "internal"), 12.0);
}

#[test]
fn loading_damaged_state_fails_without_disturbing_the_plugin() {
    let mut instance = bridge_instance();
    flush_inactive(&mut instance, &[("mix", 0.75)]);

    for bad in [
        &b""[..],
        &b"not json"[..],
        &b"{"[..],
        &br#"{"other": 1}"#[..],
        &br#"{"params": 5}"#[..],
    ] {
        assert!(
            !load_state(&mut instance, bad),
            "{:?} must be reported as a failed load",
            String::from_utf8_lossy(bad)
        );
    }

    assert_eq!(
        get_value(&mut instance, "mix"),
        0.75,
        "a failed load must change nothing"
    );
}

// ---------------------------------------------------------------------------
// state: the active (shared-atomics) path
// ---------------------------------------------------------------------------

#[test]
fn state_saved_while_active_matches_state_saved_while_inactive() {
    let mut instance = bridge_instance();
    flush_inactive(&mut instance, &[("mix", 0.375), ("taps", 5.0)]);
    let inactive_bytes = save_state(&mut instance);

    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");
    // The plugin object now lives in the audio processor: this save goes
    // through the shared atomics + TempParamOwned path instead.
    let active_bytes = save_state(&mut instance);
    instance.deactivate(processor);

    assert_eq!(
        state_json(&inactive_bytes),
        state_json(&active_bytes),
        "the active save path must produce the same JSON as the inactive one"
    );
}

#[test]
fn state_loaded_while_active_is_visible_to_the_host_immediately() {
    let mut instance = bridge_instance();
    let bytes = serde_json::to_vec(&json!({
        "params": { "mix": 0.125, "taps": 7.0, "bypass": 1.0, "internal": 18.0 }
    }))
    .unwrap();

    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");
    assert!(load_state(&mut instance, &bytes));

    // `get_value` reads the shared atomics while active — the freshly
    // loaded values must already be there, without waiting for a block.
    assert_eq!(get_value(&mut instance, "mix"), 0.125);
    assert_eq!(get_value(&mut instance, "taps"), 7.0);
    assert_eq!(get_value(&mut instance, "bypass"), 1.0);
    assert_eq!(get_value(&mut instance, "internal"), 18.0);

    instance.deactivate(processor);

    // …and they survive the trip back to the main-thread plugin object.
    assert_eq!(get_value(&mut instance, "mix"), 0.125);
    assert_eq!(get_value(&mut instance, "taps"), 7.0);
}

/// The load path that runs while the plugin is active writes the shared
/// atomics directly instead of going through `Param::set_plain`, so it has
/// to reproduce that method's guards itself. If it doesn't, reopening the
/// same project restores different values depending on whether the plugin
/// was active at the time.
#[test]
fn the_active_and_inactive_load_paths_agree_on_hostile_state() {
    let hostile = serde_json::to_vec(&json!({
        "params": {
            // Way out of range in both directions.
            "mix": 40.0,
            "internal": -1000.0,
            // Fractional value for a stepped param.
            "taps": 5.4,
            // Out-of-range boolean.
            "bypass": 3.0
        }
    }))
    .unwrap();

    let mut inactive = bridge_instance();
    assert!(load_state(&mut inactive, &hostile));

    let mut active = bridge_instance();
    let processor = active
        .activate(|_, _| (), audio_config())
        .expect("activation");
    assert!(load_state(&mut active, &hostile));

    for id in ["mix", "taps", "bypass", "internal"] {
        assert_eq!(
            get_value(&mut active, id),
            get_value(&mut inactive, id),
            "`{id}` differs between the active and inactive load paths"
        );
    }

    // And both landed on the clamped/quantized value, not the raw one.
    assert_eq!(get_value(&mut inactive, "mix"), 1.0);
    assert_eq!(get_value(&mut inactive, "taps"), 5.0);
    assert_eq!(get_value(&mut inactive, "bypass"), 1.0);
    assert_eq!(get_value(&mut inactive, "internal"), -24.0);

    active.deactivate(processor);
}

#[test]
fn a_state_saved_while_active_reloads_identically() {
    let mut instance = bridge_instance();
    flush_inactive(&mut instance, &[("mix", 0.625), ("taps", 2.0)]);

    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");
    let bytes = save_state(&mut instance);
    instance.deactivate(processor);

    let mut target = bridge_instance();
    assert!(load_state(&mut target, &bytes));
    assert_eq!(get_value(&mut target, "mix"), 0.625);
    assert_eq!(get_value(&mut target, "taps"), 2.0);
}

#[test]
fn loading_damaged_state_while_active_fails_cleanly() {
    let mut instance = bridge_instance();
    flush_inactive(&mut instance, &[("mix", 0.75)]);

    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");
    assert!(!load_state(&mut instance, b"not json"));
    assert!(!load_state(&mut instance, br#"{"other": 1}"#));
    assert_eq!(get_value(&mut instance, "mix"), 0.75);
    instance.deactivate(processor);
}

// ---------------------------------------------------------------------------
// state: extra (non-param) state
// ---------------------------------------------------------------------------

/// Extra state must survive both bridge save paths and be handed back on
/// load — this is how the IR loader and the amp's model path persist.
///
/// Everything about the extra-state plugin lives in one test because its
/// saver is process-wide shared state.
#[test]
fn extra_state_is_saved_and_restored_through_both_paths() {
    let mut instance =
        instantiate::<ExtraStatePlugin>(c"resonance-test-bridge-extra.clap", c"test.bridge-extra");

    *extra().path.lock().unwrap() = "/tmp/cab.wav".to_string();

    // Inactive save: the plugin's own `save_state` merges the extra keys.
    let inactive = state_json(&save_state(&mut instance));
    assert_eq!(inactive["ir_path"], json!("/tmp/cab.wav"));
    assert_eq!(inactive["params"]["gain"], json!(0.5));

    // Active save: the bridge rebuilds the params from shared atomics and
    // must merge the extra keys in exactly the same shape.
    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");
    let active = state_json(&save_state(&mut instance));
    assert_eq!(active, inactive, "both save paths must agree");

    // Active load hands the whole parsed state to the saver.
    *extra().loaded.lock().unwrap() = None;
    let bytes = serde_json::to_vec(&json!({
        "params": { "gain": 0.25 },
        "ir_path": "/tmp/other.wav"
    }))
    .unwrap();
    assert!(load_state(&mut instance, &bytes));
    assert_eq!(*extra().path.lock().unwrap(), "/tmp/other.wav");
    assert!(extra().loaded.lock().unwrap().is_some());
    assert_eq!(get_value(&mut instance, "gain"), 0.25);

    instance.deactivate(processor);

    // Inactive load goes through the plugin's `load_state`, and must reach
    // the saver too.
    *extra().loaded.lock().unwrap() = None;
    let bytes = serde_json::to_vec(&json!({
        "params": { "gain": 0.75 },
        "ir_path": "/tmp/third.wav"
    }))
    .unwrap();
    assert!(load_state(&mut instance, &bytes));
    assert_eq!(*extra().path.lock().unwrap(), "/tmp/third.wav");
    assert!(extra().loaded.lock().unwrap().is_some());
    assert_eq!(get_value(&mut instance, "gain"), 0.75);
}

/// The two load paths must agree for a value f32 cannot represent
/// exactly (review follow-up to ba todo #1257).
///
/// Every value in `the_active_and_inactive_load_paths_agree_on_hostile_state`
/// happens to be exactly f32-representable (0.125, 0.375, 40.0 -> 1.0),
/// so it cannot see this: the inactive path stores through
/// `FloatParam::set_plain`, which demotes to f32, while the shared path
/// stored the raw f64. `get_value` reads the atomics on both paths, so
/// the host reported -- and re-saved -- a different number depending on
/// whether the plugin happened to be active.
#[test]
fn the_load_paths_agree_on_a_value_f32_cannot_represent() {
    // 0.1 is the canonical case: 0.1f32 widened back to f64 is
    // 0.10000000149011612, so a raw-f64 store and an f32 store differ.
    let hostile = serde_json::to_vec(&json!({ "params": { "mix": 0.1 } })).unwrap();

    let mut inactive = bridge_instance();
    assert!(load_state(&mut inactive, &hostile));

    let mut active = bridge_instance();
    let processor = active
        .activate(|_, _| (), audio_config())
        .expect("activation");
    assert!(load_state(&mut active, &hostile));

    assert_eq!(
        get_value(&mut active, "mix"),
        get_value(&mut inactive, "mix"),
        "the same preset must restore the same value whether or not the \
         plugin was active when it was loaded"
    );

    active.deactivate(processor);
}
