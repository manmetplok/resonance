//! save_state -> load_state round-trip for every declared parameter and for
//! the non-param state this plugin keeps (ba todo #1340).
//!
//! The parameters are **enumerated**, not listed: a hand-written list stops
//! covering the plugin the moment someone adds a knob, and the whole point of
//! this test is that a project saved yesterday reopens with every setting the
//! user made. Each parameter is moved off its default to a value the plugin
//! itself accepted, so nothing here can pass by accidentally matching a
//! default.
//!
//! Both sides of the active/inactive boundary are covered. The first half of
//! the file drives the plugin object directly. The second half drives this
//! plugin through a real in-process CLAP host, because while the plugin is
//! **active** its object lives inside `ClapAudioProcessor` and the main thread
//! cannot reach it — the bridge serves save and load from the shared atomics
//! instead, and that is a separate code path that this plugin's parameters and
//! preset identity all ride.

use clack_extensions::params::PluginParams;
use clack_extensions::state::PluginState;
use clack_host::prelude::*;
use clack_plugin::entry::SinglePluginEntry;
use resonance_color::ResonanceColor;
use resonance_plugin::{
    stable_hash, ClapBridge, EventIterator, OutputBuffer, Param, ResonancePlugin,
};
use serde_json::{json, Value};

type Plugin = ResonanceColor;

const SAMPLE_RATE: f32 = 48_000.0;
const BLOCK: usize = 256;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A value inside `p`'s declared range that the parameter actually adopts and
/// that differs from its default, or `None` if the range holds no such value.
///
/// The range is swept rather than sampled at one point: a stepped parameter
/// quantises most fractions straight back onto its default, and a boolean only
/// has the opposite end to offer. `salt` shifts where the sweep starts so two
/// same-ranged parameters tend to land on different values — a blob where the
/// values differ is the only kind that can catch two ids reading one slot.
fn off_default(p: &dyn Param, salt: usize) -> Option<f64> {
    let (min, max, default) = (p.min_plain(), p.max_plain(), p.default_plain());
    for k in 0..16 {
        let frac = ((salt + k) % 16 + 1) as f64 / 17.0;
        p.set_plain(min + (max - min) * frac);
        if p.get_plain() != default {
            return Some(p.get_plain());
        }
    }
    p.set_plain(default);
    None
}

/// Move every declared parameter off its default, and return what each one
/// adopted in host order.
fn detune_all(plugin: &Plugin) -> Vec<(String, f64)> {
    let mut moved = Vec::new();
    let mut stuck = Vec::new();
    for i in 0..plugin.param_count() {
        let p = plugin.param(i);
        match off_default(p, i) {
            Some(v) => moved.push((p.id().to_string(), v)),
            // A parameter whose range contains nothing but its default can
            // never be observed to round-trip, so it is named rather than
            // quietly skipped.
            None => stuck.push(p.id().to_string()),
        }
    }
    assert!(
        stuck.is_empty(),
        "no in-range value differs from the default for: {stuck:?}"
    );
    moved
}

fn snapshot(plugin: &Plugin) -> Vec<(String, f64)> {
    (0..plugin.param_count())
        .map(|i| {
            let p = plugin.param(i);
            (p.id().to_string(), p.get_plain())
        })
        .collect()
}

fn state_of(plugin: &Plugin) -> Value {
    serde_json::from_slice(&plugin.save_state()).expect("save_state must emit valid JSON")
}

fn params_of(state: &Value) -> &serde_json::Map<String, Value> {
    state
        .get("params")
        .and_then(|v| v.as_object())
        .expect("state must carry a `params` object")
}

/// Run silence through the plugin, leaving it in the state a host's audio
/// thread leaves it in between blocks.
fn run_blocks(plugin: &mut Plugin, blocks: usize) {
    let mut left = vec![0.0_f32; BLOCK];
    let mut right = vec![0.0_f32; BLOCK];
    for _ in 0..blocks {
        left.fill(0.0);
        right.fill(0.0);
        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, BLOCK, &mut ev, None);
        assert!(
            left.iter().chain(right.iter()).all(|s| s.is_finite()),
            "restored state drove the DSP to a non-finite sample"
        );
    }
}

/// An instance in the condition a host keeps it in while the plugin is
/// **active**: initialized against a real sample rate and several blocks into
/// a session.
fn running() -> Plugin {
    let mut plugin = Plugin::new();
    plugin.initialize(SAMPLE_RATE, BLOCK as u32);
    run_blocks(&mut plugin, 4);
    plugin
}

/// A blob of the shape a corrupt file or a hand editor produces: every value
/// far outside the parameter's declared bounds, alternating direction.
fn hostile_state(plugin: &Plugin) -> Vec<u8> {
    let mut map = serde_json::Map::new();
    for i in 0..plugin.param_count() {
        let p = plugin.param(i);
        let v = match i % 3 {
            0 => p.max_plain() + 1.0e6,
            1 => p.min_plain() - 1.0e6,
            // In range but fractional, so a stepped parameter's rounding is
            // exercised too.
            _ => (p.min_plain() + p.max_plain()) / 2.0 + 0.5,
        };
        map.insert(p.id().to_string(), json!(v));
    }
    serde_json::to_vec(&json!({ "params": map })).unwrap()
}

// ---------------------------------------------------------------------------
// Round-trip
// ---------------------------------------------------------------------------

#[test]
fn every_declared_param_survives_a_save_load_round_trip() {
    let src = Plugin::new();
    let expected = detune_all(&src);

    let state = state_of(&src);
    assert_eq!(
        params_of(&state).len(),
        src.param_count(),
        "every declared parameter must be written, including hidden ones"
    );

    let mut dst = Plugin::new();
    assert!(dst.load_state(&serde_json::to_vec(&state).unwrap()));
    assert_eq!(snapshot(&dst), expected);
}

#[test]
fn re_saving_a_loaded_state_reproduces_it_exactly() {
    let src = Plugin::new();
    detune_all(&src);
    let first = state_of(&src);

    let mut dst = Plugin::new();
    assert!(dst.load_state(&serde_json::to_vec(&first).unwrap()));
    assert_eq!(
        state_of(&dst),
        first,
        "save -> load -> save must be a fixed point"
    );
}

// ---------------------------------------------------------------------------
// The plugin object, fresh and mid-session
// ---------------------------------------------------------------------------

/// The host saves and loads state on the main thread whether or not the plugin
/// is active, and the two go through different code: while active the plugin
/// object lives in the audio processor, so the bridge serves state from the
/// shared atomics instead. That half is driven through a real CLAP host at the
/// bottom of this file.
///
/// This half is the plugin object itself: a state blob loaded into an instance
/// that is initialized and several blocks into a session must restore exactly
/// what it restores into a fresh one. A DSP that latched a parameter at
/// `initialize` time and never re-read it would report the loaded value here
/// while playing the old one.
#[test]
fn a_state_blob_restores_the_same_values_into_a_running_and_a_fresh_instance() {
    let src = Plugin::new();
    detune_all(&src);
    let bytes = src.save_state();

    let mut fresh = Plugin::new();
    assert!(fresh.load_state(&bytes));

    let mut active = running();
    assert!(active.load_state(&bytes));

    assert_eq!(snapshot(&active), snapshot(&fresh));
    assert_eq!(state_of(&active), state_of(&fresh));

    // The restored settings then have to survive contact with the audio path.
    run_blocks(&mut active, 2);
}

/// Reopening a project must restore the same sound whether or not the plugin
/// happened to be running, right down to how an out-of-range value is pulled
/// back to the declared bounds.
#[test]
fn out_of_range_state_is_clamped_the_same_way_running_or_fresh() {
    let bytes = hostile_state(&Plugin::new());

    let mut fresh = Plugin::new();
    assert!(fresh.load_state(&bytes));

    let mut active = running();
    assert!(active.load_state(&bytes));

    assert_eq!(snapshot(&active), snapshot(&fresh));

    // …and both landed inside the declared range rather than on the raw value.
    for (i, (id, value)) in snapshot(&fresh).iter().enumerate() {
        let p = fresh.param(i);
        assert!(
            *value >= p.min_plain() && *value <= p.max_plain(),
            "`{id}` restored to {value}, outside [{}, {}]",
            p.min_plain(),
            p.max_plain()
        );
    }

    run_blocks(&mut active, 2);
}

// ---------------------------------------------------------------------------
// State versions
// ---------------------------------------------------------------------------

#[test]
fn state_written_before_the_version_field_still_restores_every_param() {
    let src = Plugin::new();
    let expected = detune_all(&src);
    let mut state = state_of(&src);
    assert!(
        state.get("version").is_some(),
        "this plugin's state carries a version field, so the pre-version \
         shape is a real case"
    );
    state.as_object_mut().unwrap().remove("version");

    let mut dst = Plugin::new();
    assert!(dst.load_state(&serde_json::to_vec(&state).unwrap()));
    assert_eq!(snapshot(&dst), expected);
}

/// Forward compatibility: a project written by a build this one has never
/// heard of must restore every parameter the two still share, rather than
/// being refused whole because of the version or the extra keys.
#[test]
fn state_from_an_unknown_future_version_still_restores_every_shared_param() {
    let src = Plugin::new();
    let expected = detune_all(&src);
    let mut state = state_of(&src);
    {
        let obj = state.as_object_mut().unwrap();
        obj.insert("version".to_string(), json!(u32::MAX));
        obj.insert("a_future_section".to_string(), json!({ "nested": true }));
    }
    state["params"]
        .as_object_mut()
        .unwrap()
        .insert("a_future_param".to_string(), json!(1.0));

    let mut dst = Plugin::new();
    assert!(dst.load_state(&serde_json::to_vec(&state).unwrap()));
    assert_eq!(snapshot(&dst), expected);
}

// ---------------------------------------------------------------------------
// Non-param state
// ---------------------------------------------------------------------------

/// The loaded-preset identity is this plugin's only state outside the params.
/// It rides at the top level of the blob beside `params`; losing it means
/// reopening a project shows a blank picker over the right sound.
#[test]
fn the_loaded_preset_identity_survives_a_round_trip() {
    let identity = json!({ "name": "Some User Preset", "source": "user", "modified": true });

    let bytes = serde_json::to_vec(&json!({ "params": {}, "preset": identity })).unwrap();

    let mut plugin = Plugin::new();
    assert!(plugin.load_state(&bytes));

    assert_eq!(state_of(&plugin).get("preset"), Some(&identity));
}

/// A project saved before preset identity existed — or with nothing loaded —
/// must leave the picker empty rather than inventing an identity.
#[test]
fn state_without_a_preset_key_leaves_no_identity_behind() {
    let mut plugin = Plugin::new();
    assert!(plugin.load_state(
        &serde_json::to_vec(&json!({
            "params": {},
            "preset": { "name": "Stale", "source": "factory", "modified": false }
        }))
        .unwrap()
    ));
    assert!(state_of(&plugin).get("preset").is_some());

    assert!(plugin.load_state(&serde_json::to_vec(&json!({ "params": {} })).unwrap()));
    assert_eq!(state_of(&plugin).get("preset"), None);
}

// ---------------------------------------------------------------------------
// The active / inactive boundary, through a real CLAP host
// ---------------------------------------------------------------------------

// Everything above holds the plugin object in hand. A host does not: once it
// activates the plugin the object moves into `ClapAudioProcessor`, and
// `state.save` / `state.load` — still [main-thread], still callable with the
// transport running — are served from the shared atomics by
// `resonance-plugin/src/clap_bridge/state.rs` instead. That second path
// rebuilds this plugin's whole document from a different source, so it is
// where a parameter or a preset identity goes missing without any of the
// tests above noticing.
//
// The framework pins the mechanism generically; the three tests below pin
// *this plugin's* state on it — its parameter set, enumerated, and the
// non-param state it keeps beside them.

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

/// This plugin behind the real CLAP C ABI, in-process.
fn hosted() -> PluginInstance<TestHost> {
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<Plugin>>>(
        c"resonance-color-state.clap",
    )
    .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();

    PluginInstance::<TestHost>::new(
        |_| TestHostShared,
        |_| (),
        &entry,
        c"com.resonance.color",
        &host_info,
    )
    .expect("plugin instantiation")
}

/// The activation config the real host uses (`clap_host/bundle.rs`).
fn audio_config() -> PluginAudioConfiguration {
    PluginAudioConfiguration {
        sample_rate: SAMPLE_RATE as f64,
        min_frames_count: 32,
        max_frames_count: 8192,
    }
}

fn state_ext(instance: &PluginInstance<TestHost>) -> PluginState {
    instance
        .plugin_shared_handle()
        .get_extension::<PluginState>()
        .expect("the bridge must expose the state extension")
}

/// What the host would write to the project file right now.
fn host_state(instance: &mut PluginInstance<TestHost>) -> Value {
    let ext = state_ext(instance);
    let mut bytes = Vec::new();
    ext.save(&mut instance.plugin_handle(), &mut bytes)
        .expect("state save");
    serde_json::from_slice(&bytes).expect("the bridge must save valid JSON")
}

fn host_load(instance: &mut PluginInstance<TestHost>, bytes: &[u8]) -> bool {
    let ext = state_ext(instance);
    ext.load(&mut instance.plugin_handle(), &mut &bytes[..])
        .is_ok()
}

/// What the host reports for one parameter, addressed by the same id hash the
/// bridge registered it under.
fn host_value(instance: &mut PluginInstance<TestHost>, id: &str) -> f64 {
    let ext = instance
        .plugin_shared_handle()
        .get_extension::<PluginParams>()
        .expect("the bridge must expose the params extension");
    ext.get_value(&mut instance.plugin_handle(), ClapId::new(stable_hash(id)))
        .unwrap_or_else(|| panic!("param `{id}` is unknown to the bridge"))
}

/// The non-param half of a saved document: everything this plugin persists
/// beside `params`, in the shape it rides in. Named once so the tests below
/// assert on the same keys the ones above do.
fn non_param_state() -> Vec<(String, Value)> {
    vec![(
        "preset".to_string(),
        json!({ "name": "Session Sound", "source": "user", "modified": true }),
    )]
}

/// A complete saved document — every parameter off its default plus the
/// non-param state — and what each parameter must come back as, in host order.
fn full_document() -> (Vec<u8>, Vec<(String, f64)>) {
    let src = Plugin::new();
    let expected = detune_all(&src);
    let mut state = state_of(&src);
    let obj = state.as_object_mut().unwrap();
    for (key, value) in non_param_state() {
        obj.insert(key, value);
    }
    (serde_json::to_vec(&state).unwrap(), expected)
}

/// Compare two saved documents key by key — and, inside `params`, parameter by
/// parameter — so a mismatch names the setting that differs instead of
/// printing two whole documents at each other.
fn assert_same_document(active: &Value, inactive: &Value) {
    fn keys(state: &Value) -> Vec<String> {
        let mut keys: Vec<String> = state
            .as_object()
            .expect("state must be a JSON object")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    assert_eq!(
        keys(active),
        keys(inactive),
        "the two save paths wrote different top-level keys"
    );

    let (active_params, inactive_params) = (params_of(active), params_of(inactive));
    let mut active_ids: Vec<&String> = active_params.keys().collect();
    let mut inactive_ids: Vec<&String> = inactive_params.keys().collect();
    active_ids.sort();
    inactive_ids.sort();
    assert_eq!(
        active_ids, inactive_ids,
        "the two save paths wrote different parameters"
    );
    for id in active_ids {
        assert_eq!(
            active_params[id], inactive_params[id],
            "`{id}` differs between the active and inactive save paths"
        );
    }

    for key in keys(active).iter().filter(|k| *k != "params") {
        assert_eq!(
            active.get(key),
            inactive.get(key),
            "`{key}` differs between the active and inactive save paths"
        );
    }
}

/// Saving with the transport running and saving with it stopped must describe
/// the same instrument, element for element — the atomics the bridge rebuilds
/// the document from have to agree with the plugin's own params, and the extra
/// keys have to be merged in the same shape by both.
#[test]
fn a_document_saved_while_active_matches_one_saved_while_inactive() {
    let (bytes, _) = full_document();

    let mut instance = hosted();
    assert!(host_load(&mut instance, &bytes));
    let inactive = host_state(&mut instance);

    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");
    // The plugin object is now inside the audio processor, so this save is
    // served from the shared atomics.
    let active = host_state(&mut instance);
    instance.deactivate(processor);

    assert_same_document(&active, &inactive);
}

/// Loading a project while the plugin is active must land every declared
/// parameter, enumerated rather than listed — the active load path writes the
/// atomics directly instead of going through `Param::set_plain`, so it is its
/// own opportunity to drop or mis-address one.
#[test]
fn a_document_loaded_while_active_lands_every_declared_param() {
    let (bytes, expected) = full_document();

    let mut instance = hosted();
    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");
    assert!(host_load(&mut instance, &bytes));

    for (id, value) in &expected {
        assert_eq!(
            host_value(&mut instance, id),
            *value,
            "`{id}` did not land on the active load path"
        );
    }

    // …and the host still reports them once the plugin goes idle again.
    instance.deactivate(processor);
    for (id, value) in &expected {
        assert_eq!(
            host_value(&mut instance, id),
            *value,
            "`{id}` was lost when the plugin was deactivated"
        );
    }
}

/// The non-param state is the part the active path handles separately from the
/// params — the bridge hands the parsed document to the extra-state saver and
/// merges its keys back on save — so it needs its own crossing of the
/// boundary. Losing it means a project reopened mid-session comes back with a
/// blank preset picker over the right sound.
#[test]
fn the_non_param_state_survives_the_active_state_path() {
    let (bytes, _) = full_document();

    let mut instance = hosted();
    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");
    assert!(host_load(&mut instance, &bytes));
    let active = host_state(&mut instance);
    instance.deactivate(processor);
    let inactive = host_state(&mut instance);

    for (key, value) in non_param_state() {
        assert_eq!(
            active.get(&key),
            Some(&value),
            "`{key}` did not survive a load and save while active"
        );
        assert_eq!(
            inactive.get(&key),
            Some(&value),
            "`{key}` was lost when the plugin was deactivated"
        );
    }
}
