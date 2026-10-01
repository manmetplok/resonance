//! The three parameter opt-outs a selector and a progress output need
//! (drums-plugin-rework.md §5.1, §5.4), checked through the real CLAP C
//! ABI:
//!
//! - `not_automatable()` — the host gets no `IS_AUTOMATABLE`, so it offers
//!   no lane (a kit swap is a multi-gigabyte decode);
//! - `excluded_from_state()` — neither written nor recalled by a state,
//!   on the inactive path or the active (shared-atomics) one;
//! - `read_only()` — CLAP `IS_READONLY`, not automatable, not saved, and
//!   a host write is ignored: only the plugin moves it.

use std::ffi::CStr;
use std::sync::Arc;

use clack_extensions::params::{ParamInfoBuffer, ParamInfoFlags, PluginParams};
use clack_extensions::state::PluginState;
use clack_host::events::event_types::ParamValueEvent;
use clack_host::prelude::*;
use clack_host::utils::Cookie;
use clack_plugin::entry::SinglePluginEntry;

use resonance_plugin::{
    stable_hash, ClapBridge, EventIterator, ExtraStateSaver, FloatParam, FloatRange, IntParam,
    IntRange, OutputBuffer, Param, ResonancePlugin, TempoInfo,
};
use serde_json::{json, Value};

struct FlagsPlugin {
    gain: FloatParam,
    /// Shared with the extra-state saver, which derives it from the
    /// state's own `pick` key — the shape of the drums' `kit_select`,
    /// which follows the state's kit reference.
    selector: Arc<IntParam>,
    progress: FloatParam,
}

impl FlagsPlugin {
    fn params(&self) -> [&dyn Param; 3] {
        [&self.gain, &*self.selector, &self.progress]
    }
}

struct PickSaver(Arc<IntParam>);

impl ExtraStateSaver for PickSaver {
    fn save(&self) -> serde_json::Map<String, Value> {
        let mut map = serde_json::Map::new();
        map.insert("pick".into(), json!(self.0.value()));
        map
    }
    fn load(&self, state: &Value) {
        if let Some(pick) = state.get("pick").and_then(Value::as_i64) {
            self.0.set_value(pick as i32);
        }
    }
}

impl ResonancePlugin for FlagsPlugin {
    const CLAP_ID: &'static str = "test.param-output-flags";
    const NAME: &'static str = "ParamOutputFlags";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static std::ffi::CStr] =
        &[resonance_plugin::features::AUDIO_EFFECT];
    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            gain: FloatParam::new(
                "gain",
                "Gain",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            selector: Arc::new(
                IntParam::new(
                    "selector",
                    "Selector",
                    -1,
                    IntRange::Linear { min: -1, max: 9 },
                )
                .not_automatable()
                .excluded_from_state(),
            ),
            progress: FloatParam::new(
                "progress",
                "Progress",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .read_only(),
        }
    }
    fn param_count(&self) -> usize {
        3
    }
    fn param(&self, index: usize) -> &dyn Param {
        self.params()[index]
    }
    fn initialize(&mut self, _sample_rate: f32, _max_buffer_size: u32) -> bool {
        // The plugin is the only writer of its output.
        self.progress.set_value(0.25);
        true
    }
    fn reset(&mut self) {}
    fn extra_state_saver(&self) -> Option<Arc<dyn ExtraStateSaver>> {
        Some(Arc::new(PickSaver(self.selector.clone())))
    }
    fn process(
        &mut self,
        _outputs: &mut [OutputBuffer<'_>],
        _frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
    }
}

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

fn instance() -> PluginInstance<TestHost> {
    let bundle: &CStr = c"resonance-test-param-output-flags.clap";
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<FlagsPlugin>>>(bundle)
        .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();
    PluginInstance::<TestHost>::new(
        |_| TestHostShared,
        |_| (),
        &entry,
        c"test.param-output-flags",
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

fn params_ext(instance: &PluginInstance<TestHost>) -> PluginParams {
    instance
        .plugin_shared_handle()
        .get_extension::<PluginParams>()
        .expect("params extension")
}

fn state_ext(instance: &PluginInstance<TestHost>) -> PluginState {
    instance
        .plugin_shared_handle()
        .get_extension::<PluginState>()
        .expect("state extension")
}

fn clap_id(id: &str) -> ClapId {
    ClapId::new(stable_hash(id))
}

fn get_value(instance: &mut PluginInstance<TestHost>, id: &str) -> f64 {
    let ext = params_ext(instance);
    ext.get_value(&mut instance.plugin_handle(), clap_id(id))
        .expect("known param")
}

fn flags_of(instance: &mut PluginInstance<TestHost>, index: u32) -> ParamInfoFlags {
    let ext = params_ext(instance);
    let mut buffer = ParamInfoBuffer::new();
    ext.get_info(&mut instance.plugin_handle(), index, &mut buffer)
        .expect("info")
        .flags
}

fn save_state(instance: &mut PluginInstance<TestHost>) -> Value {
    let ext = state_ext(instance);
    let mut bytes = Vec::new();
    ext.save(&mut instance.plugin_handle(), &mut bytes)
        .expect("state save");
    serde_json::from_slice(&bytes).expect("json")
}

fn load_state(instance: &mut PluginInstance<TestHost>, state: &Value) {
    let ext = state_ext(instance);
    let bytes = serde_json::to_vec(state).unwrap();
    ext.load(&mut instance.plugin_handle(), &mut &bytes[..])
        .expect("state load");
}

#[test]
fn the_host_sees_the_opt_outs_as_clap_flags() {
    let mut instance = instance();
    let gain = flags_of(&mut instance, 0);
    assert!(gain.contains(ParamInfoFlags::IS_AUTOMATABLE));
    assert!(!gain.contains(ParamInfoFlags::IS_READONLY));

    let selector = flags_of(&mut instance, 1);
    assert!(
        !selector.contains(ParamInfoFlags::IS_AUTOMATABLE),
        "no lane for a selector"
    );
    assert!(selector.contains(ParamInfoFlags::IS_STEPPED));
    assert!(!selector.contains(ParamInfoFlags::IS_READONLY));

    let progress = flags_of(&mut instance, 2);
    assert!(progress.contains(ParamInfoFlags::IS_READONLY));
    assert!(!progress.contains(ParamInfoFlags::IS_AUTOMATABLE));
}

/// Neither the selector nor the output is in a saved state, and a state
/// that carries them (written by hand, or before the opt-out) recalls
/// neither — on both load paths.
#[test]
fn state_neither_writes_nor_recalls_the_excluded_params() {
    let mut instance = instance();
    let saved = save_state(&mut instance);
    let params = saved["params"].as_object().expect("params map");
    assert!(params.contains_key("gain"));
    assert!(!params.contains_key("selector"), "{saved}");
    assert!(!params.contains_key("progress"), "{saved}");

    let carrying = json!({
        "version": 1,
        "params": { "gain": 0.75, "selector": 4.0, "progress": 1.0 },
    });
    // Inactive: the plugin object's own load.
    load_state(&mut instance, &carrying);
    assert_eq!(get_value(&mut instance, "gain"), 0.75);
    assert_eq!(get_value(&mut instance, "selector"), -1.0);
    assert_eq!(get_value(&mut instance, "progress"), 0.0);

    // Active: the shared-atomics path.
    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activate");
    let saved = save_state(&mut instance);
    let params = saved["params"].as_object().expect("params map");
    assert!(!params.contains_key("selector") && !params.contains_key("progress"));
    load_state(
        &mut instance,
        &json!({ "version": 1, "params": { "gain": 0.25, "selector": 7.0, "progress": 1.0 } }),
    );
    assert_eq!(get_value(&mut instance, "gain"), 0.25);
    assert_eq!(get_value(&mut instance, "selector"), -1.0);
    assert_ne!(get_value(&mut instance, "progress"), 1.0);
    instance.deactivate(processor);
}

/// A host write to the output is ignored; a write to the selector (not
/// automatable, but settable) lands.
#[test]
fn a_host_write_to_a_read_only_param_is_ignored() {
    let mut instance = instance();
    let ext = params_ext(&instance);
    let mut input = EventBuffer::new();
    for (id, value) in [("progress", 0.9), ("selector", 3.0)] {
        input.push(&ParamValueEvent::new(
            0,
            clap_id(id),
            Pckn::match_all(),
            value,
            Cookie::empty(),
        ));
    }
    let mut output = EventBuffer::new();
    let mut handle = instance.inactive_plugin_handle().expect("inactive");
    ext.flush(&mut handle, &input.as_input(), &mut output.as_output());
    assert_eq!(get_value(&mut instance, "progress"), 0.0);
    assert_eq!(get_value(&mut instance, "selector"), 3.0);
}

/// A param the plugin derives from the state it is loading (the drums'
/// `kit_select` from its kit reference) is not reverted by the load's
/// own shared → plugin re-sync: it was not part of the params the load
/// stored, so the stale atomic must not win over what the plugin set.
#[test]
fn a_load_while_active_keeps_the_value_the_plugin_derived_from_it() {
    let mut instance = instance();
    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activate");
    load_state(
        &mut instance,
        &json!({ "version": 1, "params": {}, "pick": 5 }),
    );
    // The deactivation reconciles the two sides with the load still
    // unapplied (no block ran): the derived value stands.
    instance.deactivate(processor);
    assert_eq!(get_value(&mut instance, "selector"), 5.0);
}

/// A `Param` implementation that opts out of the state alone still stays
/// out of presets: the trait's default `preset_excluded` follows
/// `state_excluded`, so no implementor can leave a state-excluded value
/// in a preset by forgetting the second override.
#[test]
fn state_excluded_implies_preset_excluded_for_any_implementor() {
    struct OnlyStateExcluded;
    impl Param for OnlyStateExcluded {
        fn id(&self) -> &str {
            "only"
        }
        fn name(&self) -> &str {
            "Only"
        }
        fn get_plain(&self) -> f64 {
            0.0
        }
        fn set_plain(&self, _v: f64) {}
        fn default_plain(&self) -> f64 {
            0.0
        }
        fn min_plain(&self) -> f64 {
            0.0
        }
        fn max_plain(&self) -> f64 {
            1.0
        }
        fn display(&self, value: f64) -> String {
            value.to_string()
        }
        fn parse(&self, text: &str) -> Option<f64> {
            text.parse().ok()
        }
        fn state_excluded(&self) -> bool {
            true
        }
    }
    assert!(OnlyStateExcluded.preset_excluded());

    // The builders agree, whichever order they are called in.
    let read_only = FloatParam::new("p", "P", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
        .read_only();
    assert!(read_only.state_excluded() && read_only.preset_excluded());
}

/// An active-path `flush` stores the value the param LANDED on — clamped
/// to its range, rounded for an int — not the raw wire value, so the
/// host never reads back (or re-saves) a number the plugin did not take.
#[test]
fn an_active_flush_mirrors_the_landed_value_not_the_wire_value() {
    let mut instance = instance();
    let ext = params_ext(&instance);
    let mut processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activate");
    let mut input = EventBuffer::new();
    for (id, value) in [("gain", 7.5), ("selector", 3.4)] {
        input.push(&ParamValueEvent::new(
            0,
            clap_id(id),
            Pckn::match_all(),
            value,
            Cookie::empty(),
        ));
    }
    let mut output = EventBuffer::new();
    ext.flush_active(
        &mut processor.plugin_handle(),
        &input.as_input(),
        &mut output.as_output(),
    );
    assert_eq!(get_value(&mut instance, "gain"), 1.0, "clamped to max");
    assert_eq!(get_value(&mut instance, "selector"), 3.0, "rounded");
    instance.deactivate(processor);
}

/// The bridge publishes which params its state leaves out through
/// `com.resonance.param-flags`, so the host leaves them out of what it
/// persists: CLAP's own flags have no bit for it.
#[test]
fn the_bridge_publishes_state_excluded_params() {
    use resonance_common::param_flags::{PluginParamFlags, EXTENSION_ID};
    let instance = instance();
    let raw = instance.raw_instance();
    let ext = unsafe { (raw.get_extension.expect("get_extension"))(raw, EXTENSION_ID.as_ptr()) }
        as *const PluginParamFlags;
    assert!(!ext.is_null(), "a Resonance plugin serves the extension");
    let is_excluded = unsafe { (*ext).is_state_excluded.expect("is_state_excluded") };
    let ask = |id: &str| unsafe {
        is_excluded(
            raw as *const _ as *const std::ffi::c_void,
            stable_hash(id),
        )
    };
    assert!(!ask("gain"));
    assert!(ask("selector"), "excluded_from_state()");
    assert!(ask("progress"), "read_only() implies it");
    assert!(!ask("no-such-param"));
}
