//! Unit tests for the preset / project state format (`src/state.rs`) and
//! the `ResonancePlugin` save/load defaults that sit on top of it.
//!
//! This JSON shape — `{"params": {"<id>": <number>, …}}` plus any
//! top-level extra-state keys — is what all twelve plugins write into every
//! user project file. A change here silently rewrites or drops saved
//! settings in projects that already exist on disk, so the round-trip and
//! the tolerated-garbage behaviour are pinned deliberately.

use std::sync::{Arc, Mutex};

use resonance_plugin::state::{load_params_from_json, params_to_json};
use resonance_plugin::{
    BoolParam, EventIterator, ExtraStateSaver, FloatParam, FloatRange, IntParam, IntRange,
    OutputBuffer, Param, ResonancePlugin, TempoInfo,
};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// A representative param set: one continuous, one stepped, one boolean, one
// hidden — the same mix a real plugin declares.
// ---------------------------------------------------------------------------

struct TestParams {
    mix: FloatParam,
    taps: IntParam,
    bypass: BoolParam,
    internal: FloatParam,
}

impl TestParams {
    fn new() -> Self {
        Self {
            mix: FloatParam::new("mix", "Mix", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 }),
            taps: IntParam::new("taps", "Taps", 3, IntRange::Linear { min: 1, max: 8 }),
            bypass: BoolParam::new("bypass", "Bypass", false),
            internal: FloatParam::new(
                "internal",
                "Internal",
                -6.0,
                FloatRange::Linear {
                    min: -24.0,
                    max: 24.0,
                },
            )
            .hidden(),
        }
    }

    fn refs(&self) -> Vec<&dyn Param> {
        vec![&self.mix, &self.taps, &self.bypass, &self.internal]
    }
}

fn params_object(state: &Value) -> &serde_json::Map<String, Value> {
    state
        .get("params")
        .and_then(|v| v.as_object())
        .expect("state must carry a `params` object")
}

// ---------------------------------------------------------------------------
// params_to_json
// ---------------------------------------------------------------------------

#[test]
fn params_to_json_writes_every_param_under_a_params_object() {
    let p = TestParams::new();
    let state = params_to_json(&p.refs());

    let map = params_object(&state);
    assert_eq!(map.len(), 4, "hidden params are persisted too");
    assert_eq!(map["mix"], json!(0.5));
    assert_eq!(map["taps"], json!(3.0));
    assert_eq!(map["bypass"], json!(0.0));
    assert_eq!(map["internal"], json!(-6.0));

    // Nothing else lands at the top level.
    let top = state.as_object().expect("state must be an object");
    assert_eq!(top.len(), 1);
    assert!(top.contains_key("params"));
}

#[test]
fn params_to_json_snapshots_current_values_not_defaults() {
    let p = TestParams::new();
    p.mix.set_plain(0.25);
    p.taps.set_plain(7.0);
    p.bypass.set_plain(1.0);

    let map = params_to_json(&p.refs()).get("params").cloned().unwrap();
    assert_eq!(map["mix"], json!(0.25));
    assert_eq!(map["taps"], json!(7.0));
    assert_eq!(map["bypass"], json!(1.0));
}

#[test]
fn an_empty_param_list_still_produces_a_valid_state() {
    let state = params_to_json(&[]);
    assert!(params_object(&state).is_empty());
    // …and loading it back is a success, not a failure.
    assert!(load_params_from_json(&[], &state));
}

// ---------------------------------------------------------------------------
// Round-trip
// ---------------------------------------------------------------------------

#[test]
fn params_survive_a_save_load_round_trip() {
    let saved = TestParams::new();
    saved.mix.set_plain(0.125);
    saved.taps.set_plain(6.0);
    saved.bypass.set_plain(1.0);
    saved.internal.set_plain(11.5);
    let state = params_to_json(&saved.refs());

    // A fresh instance sits at its defaults until the state is applied.
    let loaded = TestParams::new();
    assert_eq!(loaded.mix.value(), 0.5);
    assert!(load_params_from_json(&loaded.refs(), &state));

    assert_eq!(loaded.mix.value(), 0.125);
    assert_eq!(loaded.taps.value(), 6);
    assert!(loaded.bypass.value());
    assert_eq!(loaded.internal.value(), 11.5);
}

#[test]
fn a_round_trip_through_serialized_bytes_is_lossless() {
    let saved = TestParams::new();
    saved.mix.set_plain(0.3);
    saved.taps.set_plain(2.0);

    let bytes = serde_json::to_vec(&params_to_json(&saved.refs())).unwrap();
    let reparsed: Value = serde_json::from_slice(&bytes).unwrap();

    let loaded = TestParams::new();
    assert!(load_params_from_json(&loaded.refs(), &reparsed));
    assert_eq!(loaded.mix.value(), saved.mix.value());
    assert_eq!(loaded.taps.value(), saved.taps.value());
}

#[test]
fn re_saving_a_loaded_state_reproduces_it_exactly() {
    let original = TestParams::new();
    original.mix.set_plain(0.75);
    original.taps.set_plain(8.0);
    original.bypass.set_plain(1.0);
    let first = params_to_json(&original.refs());

    let loaded = TestParams::new();
    load_params_from_json(&loaded.refs(), &first);
    let second = params_to_json(&loaded.refs());

    assert_eq!(first, second, "save -> load -> save must be a fixed point");
}

// ---------------------------------------------------------------------------
// Tolerating older / newer / damaged state
// ---------------------------------------------------------------------------

#[test]
fn a_missing_key_leaves_that_param_untouched() {
    // The shape of an older project saved before `taps` existed.
    let state = json!({ "params": { "mix": 0.9, "bypass": 1.0, "internal": 3.0 } });

    let p = TestParams::new();
    p.taps.set_plain(5.0);
    assert!(load_params_from_json(&p.refs(), &state));

    assert_eq!(p.mix.value(), 0.9);
    assert_eq!(
        p.taps.value(),
        5,
        "a param absent from the state must not move"
    );
    assert!(p.bypass.value());
}

#[test]
fn unknown_keys_are_ignored() {
    // The shape of a project saved by a *newer* build with extra params.
    let state = json!({
        "params": { "mix": 0.2, "some_future_param": 12.0 },
        "some_future_section": { "nested": true }
    });

    let p = TestParams::new();
    assert!(load_params_from_json(&p.refs(), &state));
    assert_eq!(p.mix.value(), 0.2);
    assert_eq!(p.taps.value(), 3, "defaults survive an unknown-key load");
}

#[test]
fn wrong_typed_values_are_skipped_without_panicking() {
    let state = json!({
        "params": {
            "mix": "0.9",          // string instead of number
            "taps": null,
            "bypass": true,        // JSON bool, not the 0.0/1.0 encoding
            "internal": { "value": 3.0 }
        }
    });

    let p = TestParams::new();
    p.mix.set_plain(0.4);
    p.taps.set_plain(6.0);
    p.bypass.set_plain(1.0);
    p.internal.set_plain(2.0);

    assert!(load_params_from_json(&p.refs(), &state));

    // Every param keeps the value it had — a mistyped entry must not
    // reset a knob to zero or to its default.
    assert_eq!(p.mix.value(), 0.4);
    assert_eq!(p.taps.value(), 6);
    assert!(p.bypass.value());
    assert_eq!(p.internal.value(), 2.0);
}

#[test]
fn out_of_range_values_are_clamped_on_load() {
    // A hand-edited or corrupted preset must not push a param past the
    // bounds its DSP was written against.
    let state = json!({
        "params": { "mix": 40.0, "taps": -12.0, "internal": 1e30 }
    });

    let p = TestParams::new();
    assert!(load_params_from_json(&p.refs(), &state));

    assert_eq!(p.mix.value(), 1.0);
    assert_eq!(p.taps.value(), 1);
    assert_eq!(p.internal.value(), 24.0);
}

#[test]
fn state_without_a_params_object_is_rejected() {
    let p = TestParams::new();

    for bad in [
        json!({}),
        json!({ "other": 1 }),
        json!({ "params": 5 }),
        json!({ "params": [1, 2, 3] }),
        json!({ "params": null }),
        json!(null),
        json!(42),
        json!("params"),
    ] {
        assert!(
            !load_params_from_json(&p.refs(), &bad),
            "{bad} must be reported as a failed load"
        );
    }

    // …and nothing was applied along the way.
    assert_eq!(p.mix.value(), 0.5);
    assert_eq!(p.taps.value(), 3);
}

#[test]
fn an_empty_params_object_loads_successfully_and_changes_nothing() {
    let p = TestParams::new();
    p.mix.set_plain(0.8);

    assert!(load_params_from_json(&p.refs(), &json!({ "params": {} })));
    assert_eq!(p.mix.value(), 0.8);
}

// ---------------------------------------------------------------------------
// The ResonancePlugin save_state / load_state defaults
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ExtraState {
    ir_path: Mutex<String>,
}

impl ExtraStateSaver for ExtraState {
    fn save(&self) -> serde_json::Map<String, Value> {
        let mut map = serde_json::Map::new();
        map.insert(
            "ir_path".to_string(),
            json!(self.ir_path.lock().unwrap().clone()),
        );
        map
    }

    fn load(&self, state: &Value) {
        if let Some(path) = state.get("ir_path").and_then(|v| v.as_str()) {
            *self.ir_path.lock().unwrap() = path.to_string();
        }
    }
}

struct StatePlugin {
    params: TestParams,
    extra: Option<Arc<ExtraState>>,
}

impl StatePlugin {
    fn with_extra() -> Self {
        Self {
            params: TestParams::new(),
            extra: Some(Arc::new(ExtraState::default())),
        }
    }
}

impl ResonancePlugin for StatePlugin {
    const CLAP_ID: &'static str = "test.state";
    const NAME: &'static str = "State";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static std::ffi::CStr] =
        &[resonance_plugin::features::AUDIO_EFFECT];
    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: TestParams::new(),
            extra: None,
        }
    }
    fn param_count(&self) -> usize {
        4
    }
    fn param(&self, index: usize) -> &dyn Param {
        self.params.refs()[index]
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
        self.extra
            .as_ref()
            .map(|e| e.clone() as Arc<dyn ExtraStateSaver>)
    }
}

#[test]
fn the_default_save_state_emits_the_documented_json_shape() {
    let mut plugin = StatePlugin::new();
    plugin.params.mix.set_plain(0.25);

    let bytes = plugin.save_state();
    let state: Value = serde_json::from_slice(&bytes).expect("save_state must emit valid JSON");
    assert_eq!(params_object(&state)["mix"], json!(0.25));

    // And it round-trips back through the default load_state.
    plugin.params.mix.set_plain(0.0);
    assert!(plugin.load_state(&bytes));
    assert_eq!(plugin.params.mix.value(), 0.25);
}

#[test]
fn the_default_load_state_rejects_bytes_that_are_not_state() {
    let mut plugin = StatePlugin::new();

    assert!(!plugin.load_state(b""), "empty input is not valid state");
    assert!(!plugin.load_state(b"not json at all"));
    assert!(!plugin.load_state(b"{"), "truncated JSON must fail");
    assert!(
        !plugin.load_state(b"{\"other\":1}"),
        "valid JSON without params must fail"
    );

    // A rejected load leaves the plugin exactly as it was.
    assert_eq!(plugin.params.mix.value(), 0.5);
}

#[test]
fn extra_state_is_merged_at_the_top_level_alongside_params() {
    let plugin = StatePlugin::with_extra();
    *plugin.extra.as_ref().unwrap().ir_path.lock().unwrap() = "/tmp/cab.wav".to_string();
    plugin.params.mix.set_plain(0.375);

    let bytes = plugin.save_state();
    let state: Value = serde_json::from_slice(&bytes).unwrap();

    // Both halves are present, and the extra key sits beside `params`
    // rather than inside it.
    assert_eq!(state["ir_path"], json!("/tmp/cab.wav"));
    assert_eq!(params_object(&state)["mix"], json!(0.375));
    assert!(!params_object(&state).contains_key("ir_path"));
}

#[test]
fn extra_state_round_trips_through_load_state() {
    let source = StatePlugin::with_extra();
    *source.extra.as_ref().unwrap().ir_path.lock().unwrap() = "/tmp/cab.wav".to_string();
    source.params.taps.set_plain(8.0);
    let bytes = source.save_state();

    let mut target = StatePlugin::with_extra();
    assert!(target.load_state(&bytes));

    assert_eq!(
        *target.extra.as_ref().unwrap().ir_path.lock().unwrap(),
        "/tmp/cab.wav"
    );
    assert_eq!(target.params.taps.value(), 8);
}

#[test]
fn a_plugin_without_extra_state_can_still_load_state_that_has_some() {
    // A project saved by a build whose plugin had extra state, opened by
    // one that doesn't: the params must still come through.
    let source = StatePlugin::with_extra();
    *source.extra.as_ref().unwrap().ir_path.lock().unwrap() = "/tmp/cab.wav".to_string();
    source.params.mix.set_plain(0.9);
    let bytes = source.save_state();

    let mut plain = StatePlugin::new();
    assert!(plain.load_state(&bytes));
    assert_eq!(plain.params.mix.value(), 0.9);
}
