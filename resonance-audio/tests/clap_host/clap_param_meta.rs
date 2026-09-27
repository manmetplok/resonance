//! What the host learns about a parameter beyond its number
//! (ba todo #1290, finding X8).
//!
//! Two halves:
//!
//! - the pure readers ([`unit_from_text`], [`choice_labels`]) — CLAP has
//!   no unit field and no enum list, so both are recovered from the
//!   plugin's own `value_to_text`, and how they decide "there is nothing
//!   here" matters as much as what they extract;
//! - `query_params` across the real C boundary, against a hand-rolled
//!   fake plugin built from `clap_sys` vtables (the harness
//!   `clap_latency_tracking.rs` uses). A stepped parameter that names
//!   its steps must arrive with its labels, a unit-carrying one with its
//!   unit, a grouped one with its module, and a hidden one must be
//!   present and flagged rather than dropped — it is still automatable
//!   and still saved.

use std::ffi::{c_char, c_void, CStr};
use std::ptr;

use clap_sys::ext::params::{
    clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS, CLAP_PARAM_IS_HIDDEN,
    CLAP_PARAM_IS_STEPPED,
};
use clap_sys::plugin::clap_plugin;

use indexmap::IndexMap;
use resonance_audio::test_support::{
    choice_labels, AutomationSnapshot, ClapInstance, PluginSlot, __instance_from_raw_for_test,
};
use resonance_audio::unit_from_text;
use resonance_audio::AutomationLanes;
use resonance_common::automation::{AutomationLane, AutomationTarget, Breakpoint, CurveKind};

// ---------------------------------------------------------------------------
// unit_from_text
// ---------------------------------------------------------------------------

#[test]
fn unit_is_what_follows_the_number() {
    assert_eq!(unit_from_text("-6.0 dB"), "dB");
    assert_eq!(unit_from_text("40 %"), "%");
    assert_eq!(unit_from_text("40%"), "%");
    assert_eq!(unit_from_text("375.0 ms"), "ms");
    assert_eq!(unit_from_text("1200 Hz"), "Hz");
    assert_eq!(unit_from_text("  0.71 Q  "), "Q");
    assert_eq!(unit_from_text("+3 st"), "st");
}

#[test]
fn a_name_is_not_a_unit() {
    // The display of a choice parameter. Calling "Low-pass" the unit of
    // a filter-type parameter would be worse than reporting none.
    assert_eq!(unit_from_text("Low-pass"), "");
    // A note division begins with a number without being one: "/8D" is
    // not the unit of anything.
    assert_eq!(unit_from_text("1/8D"), "");
    assert_eq!(unit_from_text("3 voices in unison"), "");
    assert_eq!(unit_from_text("-inf dB"), "");
    assert_eq!(unit_from_text(""), "");
}

#[test]
fn a_bare_number_has_no_unit() {
    assert_eq!(unit_from_text("3"), "");
    assert_eq!(unit_from_text("-0.50"), "");
}

#[test]
fn an_exponent_belongs_to_the_number() {
    assert_eq!(unit_from_text("1.5e-3 s"), "s");
    assert_eq!(unit_from_text("2e4"), "");
}

// ---------------------------------------------------------------------------
// choice_labels
// ---------------------------------------------------------------------------

#[test]
fn labels_are_collected_from_the_minimum_up() {
    let names = ["Low-pass", "Band-pass", "High-pass"];
    let labels = choice_labels(0.0, 2.0, |v| Some(names[v as usize].to_string()))
        .expect("a named stepped parameter is a choice parameter");
    assert_eq!(labels, vec!["Low-pass", "Band-pass", "High-pass"]);
}

#[test]
fn labels_are_indexed_from_a_non_zero_minimum() {
    // `choices[0]` is the label AT the minimum, whatever the minimum is.
    let labels = choice_labels(1.0, 3.0, |v| Some(format!("Mode {v}")))
        .expect("named steps");
    assert_eq!(labels, vec!["Mode 1", "Mode 2", "Mode 3"]);
}

#[test]
fn a_count_is_not_a_choice_parameter() {
    // "Voices 1..8" formats step 3 as "3": listing those as choices
    // would dress a count up as an enumeration.
    assert!(choice_labels(1.0, 8.0, |v| Some(format!("{v}"))).is_none());
}

#[test]
fn one_informative_label_is_enough() {
    // A count whose zero means something ("Off, 1, 2, ...") is still
    // worth naming.
    let labels = choice_labels(0.0, 3.0, |v| {
        Some(if v == 0.0 {
            "Off".to_string()
        } else {
            format!("{v}")
        })
    })
    .expect("the Off label carries meaning the number does not");
    assert_eq!(labels, vec!["Off", "1", "2", "3"]);
}

#[test]
fn an_unformattable_or_oversized_range_is_no_choice_list() {
    // A plugin that cannot format a step: report nothing rather than a
    // table with a hole in it.
    assert!(choice_labels(0.0, 3.0, |_| None).is_none());
    assert!(choice_labels(0.0, 3.0, |_| Some("  ".to_string())).is_none());
    // A MIDI-note-sized range: one `value_to_text` call per step, for a
    // list nobody reads as an enumeration.
    assert!(choice_labels(0.0, 127.0, |v| Some(format!("Note {v}"))).is_none());
    // Degenerate ranges.
    assert!(choice_labels(1.0, 1.0, |v| Some(format!("{v}"))).is_none());
    assert!(choice_labels(4.0, 0.0, |v| Some(format!("{v}"))).is_none());
    assert!(choice_labels(f64::NAN, 3.0, |v| Some(format!("{v}"))).is_none());
}

// ---------------------------------------------------------------------------
// Fake plugin: four parameters, one of each kind that matters here
// ---------------------------------------------------------------------------

const P_FILTER: u32 = 10;
const P_MIX: u32 = 11;
const P_VOICES: u32 = 12;
const P_INTERNAL: u32 = 13;

const FILTER_MODES: [&str; 3] = ["Low-pass", "Band-pass", "High-pass"];

struct FakeState {
    filter: f64,
    /// How many times the host asked the plugin to FORMAT a value.
    ///
    /// The point of counting: `query_params` is allowed to be expensive
    /// (it collects a parameter's whole meaning, once, at instantiation)
    /// but the automation path is not — it resolves a lane's range on
    /// every breakpoint edit, holding a lock the audio thread drops a
    /// block rather than wait for (ba todo #1290 review).
    text_calls: u32,
}

unsafe fn fake_state<'a>(plugin: *const clap_plugin) -> &'a mut FakeState {
    &mut *((*plugin).plugin_data as *mut FakeState)
}

unsafe extern "C" fn fake_init(_plugin: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn fake_destroy(_plugin: *const clap_plugin) {}
unsafe extern "C" fn fake_activate(
    _plugin: *const clap_plugin,
    _sample_rate: f64,
    _min_frames: u32,
    _max_frames: u32,
) -> bool {
    true
}
unsafe extern "C" fn fake_deactivate(_plugin: *const clap_plugin) {}
unsafe extern "C" fn fake_start_processing(_plugin: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn fake_stop_processing(_plugin: *const clap_plugin) {}

unsafe extern "C" fn fake_param_count(_plugin: *const clap_plugin) -> u32 {
    4
}

/// Write `s` into a CLAP fixed-size c_char array field.
unsafe fn write_cstr(dst: *mut c_char, capacity: usize, s: &str) {
    let bytes = s.as_bytes();
    let n = bytes.len().min(capacity - 1);
    ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, dst, n);
    *dst.add(n) = 0;
}

unsafe extern "C" fn fake_get_info(
    _plugin: *const clap_plugin,
    index: u32,
    out: *mut clap_param_info,
) -> bool {
    let info = &mut *out;
    info.cookie = ptr::null_mut();
    info.flags = 0;
    write_cstr(info.module.as_mut_ptr(), info.module.len(), "");
    match index {
        0 => {
            info.id = P_FILTER;
            write_cstr(info.name.as_mut_ptr(), info.name.len(), "Filter Type");
            write_cstr(info.module.as_mut_ptr(), info.module.len(), "Voice/Filter");
            info.flags = CLAP_PARAM_IS_STEPPED;
            info.min_value = 0.0;
            info.max_value = 2.0;
            info.default_value = 0.0;
        }
        1 => {
            info.id = P_MIX;
            write_cstr(info.name.as_mut_ptr(), info.name.len(), "Mix");
            info.min_value = 0.0;
            info.max_value = 1.0;
            info.default_value = 0.4;
        }
        2 => {
            info.id = P_VOICES;
            write_cstr(info.name.as_mut_ptr(), info.name.len(), "Voices");
            info.flags = CLAP_PARAM_IS_STEPPED;
            info.min_value = 1.0;
            info.max_value = 8.0;
            info.default_value = 4.0;
        }
        3 => {
            info.id = P_INTERNAL;
            write_cstr(info.name.as_mut_ptr(), info.name.len(), "Internal Trim");
            info.flags = CLAP_PARAM_IS_HIDDEN;
            info.min_value = -1.0;
            info.max_value = 1.0;
            info.default_value = 0.0;
        }
        _ => return false,
    }
    true
}

unsafe extern "C" fn fake_get_value(
    plugin: *const clap_plugin,
    param_id: u32,
    out: *mut f64,
) -> bool {
    *out = match param_id {
        P_FILTER => fake_state(plugin).filter,
        P_MIX => 0.4,
        P_VOICES => 4.0,
        P_INTERNAL => 0.0,
        _ => return false,
    };
    true
}

unsafe extern "C" fn fake_value_to_text(
    plugin: *const clap_plugin,
    param_id: u32,
    value: f64,
    out: *mut c_char,
    capacity: u32,
) -> bool {
    fake_state(plugin).text_calls += 1;
    let text = match param_id {
        P_FILTER => match FILTER_MODES.get(value.round() as usize) {
            Some(label) => (*label).to_string(),
            None => return false,
        },
        P_MIX => format!("{:.0} %", value * 100.0),
        P_VOICES => format!("{}", value.round() as i64),
        P_INTERNAL => format!("{value:.2} dB"),
        _ => return false,
    };
    write_cstr(out, capacity as usize, &text);
    true
}

static FAKE_PARAMS_EXT: clap_plugin_params = clap_plugin_params {
    count: Some(fake_param_count),
    get_info: Some(fake_get_info),
    get_value: Some(fake_get_value),
    value_to_text: Some(fake_value_to_text),
    text_to_value: None,
    flush: None,
};

unsafe extern "C" fn fake_get_extension(
    _plugin: *const clap_plugin,
    id: *const c_char,
) -> *const c_void {
    if CStr::from_ptr(id).to_bytes() == CLAP_EXT_PARAMS.to_bytes() {
        return &FAKE_PARAMS_EXT as *const clap_plugin_params as *const c_void;
    }
    ptr::null()
}

/// A `ClapInstance` around the fake plugin above. Plugin and state are
/// leaked on purpose: `Drop` still dereferences them.
fn make_instance() -> ClapInstance {
    make_instance_with_state().0
}

/// The same, plus a raw pointer to the plugin's backing state so a test
/// can read how often it was asked to format a value.
fn make_instance_with_state() -> (ClapInstance, *mut FakeState) {
    let mut state_ptr: *mut FakeState = ptr::null_mut();
    let instance = __instance_from_raw_for_test(
        |_host| {
            let state = Box::into_raw(Box::new(FakeState {
                filter: 1.0,
                text_calls: 0,
            }));
            state_ptr = state;
            let plugin = Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: state as *mut c_void,
                init: Some(fake_init),
                destroy: Some(fake_destroy),
                activate: Some(fake_activate),
                deactivate: Some(fake_deactivate),
                start_processing: Some(fake_start_processing),
                stop_processing: Some(fake_stop_processing),
                reset: None,
                process: None,
                get_extension: Some(fake_get_extension),
                on_main_thread: None,
            });
            Box::into_raw(plugin) as *const clap_plugin
        },
        48_000,
    )
    .expect("fake plugin instance");
    (instance, state_ptr)
}

// ---------------------------------------------------------------------------
// query_params across the C boundary
// ---------------------------------------------------------------------------

#[test]
fn a_choice_parameter_arrives_named() {
    let instance = make_instance();
    let params = instance.query_params();
    let filter = params
        .iter()
        .find(|p| p.id == P_FILTER)
        .expect("filter param");

    // Without these the value 1.0 on a 0..=2 range says nothing at all.
    assert!(filter.stepped);
    assert_eq!(filter.text, "Band-pass");
    assert_eq!(filter.choices, vec!["Low-pass", "Band-pass", "High-pass"]);
    assert_eq!(filter.module, "Voice/Filter");
    assert_eq!(filter.unit, "", "a mode name carries no unit");
    assert!(!filter.hidden);
}

#[test]
fn a_continuous_parameter_arrives_with_its_unit() {
    let instance = make_instance();
    let params = instance.query_params();
    let mix = params.iter().find(|p| p.id == P_MIX).expect("mix param");

    // The plugin's formatter says "40 %" where the raw value is 0.4 —
    // the whole point of finding X8.
    assert_eq!(mix.text, "40 %");
    assert_eq!(mix.unit, "%");
    assert!(!mix.stepped);
    assert!(mix.choices.is_empty());
    assert_eq!(mix.module, "");
}

#[test]
fn a_stepped_count_reports_no_choices() {
    let instance = make_instance();
    let params = instance.query_params();
    let voices = params
        .iter()
        .find(|p| p.id == P_VOICES)
        .expect("voices param");
    assert!(voices.stepped);
    assert!(
        voices.choices.is_empty(),
        "steps that format as their own number are a count, not an enumeration: {:?}",
        voices.choices
    );
    assert_eq!(voices.text, "4");
}

#[test]
fn a_hidden_parameter_is_reported_and_flagged_not_dropped() {
    let instance = make_instance();
    let params = instance.query_params();
    let internal = params
        .iter()
        .find(|p| p.id == P_INTERNAL)
        .expect("a hidden parameter is still automatable and still saved, so it stays in the list");
    assert!(internal.hidden);
    assert_eq!(internal.unit, "dB");
    // The other three are not swept up with it.
    assert_eq!(params.iter().filter(|p| p.hidden).count(), 1);
    assert_eq!(params.len(), 4);
}

#[test]
fn param_text_formats_any_value_not_just_the_current_one() {
    let instance = make_instance();
    assert_eq!(instance.param_text(P_FILTER, 2.0).as_deref(), Some("High-pass"));
    assert_eq!(instance.param_text(P_MIX, 0.75).as_deref(), Some("75 %"));
    // A value the plugin refuses to format, and an unknown parameter.
    assert_eq!(instance.param_text(P_FILTER, 9.0), None);
    assert_eq!(instance.param_text(999, 0.0), None);
}

// ---------------------------------------------------------------------------
// The cheap path: a range without a formatter
// ---------------------------------------------------------------------------

/// Drive the real `AutomationSnapshot::build` over one plugin-param
/// lane on the fake plugin, and report how many times the plugin was
/// asked to format a value along the way.
fn snapshot_for_one_lane(
    instance: ClapInstance,
    state: *mut FakeState,
    param_id: u32,
) -> (AutomationSnapshot, u32) {
    let state = unsafe { &mut *state };
    state.text_calls = 0;

    // #1304 replaced the bare `Mutex<SyncClapInstance>` in the plugin map with
    // `PluginSlot` (which derefs to that mutex, so every call site reads the same).
    let mut plugins: IndexMap<u64, std::sync::Arc<PluginSlot>> = IndexMap::new();
    plugins.insert(7, std::sync::Arc::new(PluginSlot::new(instance)));

    let mut lanes: AutomationLanes = AutomationLanes::new();
    let target = AutomationTarget::PluginParam {
        instance: 7,
        param_id,
    };
    lanes.insert(
        target.clone(),
        AutomationLane::new(
            1,
            target,
            vec![Breakpoint::new(0, 0.25, CurveKind::Linear)],
        ),
    );

    let snapshot = AutomationSnapshot::build(&lanes, &plugins);
    (snapshot, state.text_calls)
}

#[test]
fn the_automation_snapshot_resolves_a_range_without_formatting_anything() {
    // `build` runs on every SetAutomationLane — which is what a
    // breakpoint drag emits — while holding the instance lock the audio
    // thread abandons a block rather than wait for. Resolving the range
    // through `query_params` would format every parameter of the plugin,
    // per lane, per drag event, and throw all of it away (ba todo #1290
    // review).
    let (instance, state) = make_instance_with_state();
    let (snapshot, text_calls) = snapshot_for_one_lane(instance, state, P_MIX);

    let resolved = snapshot
        .plugin_params
        .get(&7)
        .and_then(|lanes| lanes.first())
        .expect("the lane resolved against the plugin's declared range");
    assert_eq!((resolved.min, resolved.max), (0.0, 1.0));
    assert_eq!(
        text_calls, 0,
        "the snapshot wanted two numbers and must not have paid for the \
         plugin's formatter to get them"
    );
}

#[test]
fn a_range_lookup_never_asks_the_plugin_to_format_anything() {
    // `AutomationSnapshot::build` resolves a lane's min/max on every
    // SetAutomationLane — which is what a breakpoint drag emits — while
    // holding the instance lock the audio thread abandons a block rather
    // than wait for. Going through `query_params` there would format
    // every parameter of the plugin, per lane, per drag event, and throw
    // all of it away (ba todo #1290 review).
    let (instance, state) = make_instance_with_state();
    let state = unsafe { &mut *state };
    state.text_calls = 0;

    assert_eq!(instance.param_range(P_MIX), Some((0.0, 1.0)));
    assert_eq!(instance.param_range(P_FILTER), Some((0.0, 2.0)));
    assert_eq!(
        state.text_calls, 0,
        "a range lookup must not call value_to_text — it is the expensive \
         thing this accessor exists to avoid"
    );

    // An unknown id is None, not a panic and not a guess.
    assert_eq!(instance.param_range(999), None);
    assert_eq!(state.text_calls, 0);
}

#[test]
fn collecting_a_parameters_meaning_is_what_costs_the_formatter_calls() {
    // The counterpart: `query_params` DOES format, on purpose, once per
    // instantiation. Asserting it here is what makes the previous test
    // mean something — otherwise a formatter that was never called at
    // all would pass both.
    let (instance, state) = make_instance_with_state();
    let state = unsafe { &mut *state };
    state.text_calls = 0;

    let params = instance.query_params();
    assert_eq!(params.len(), 4);
    assert!(
        state.text_calls >= 4,
        "one formatting call per parameter at minimum, plus the choice \
         walk; got {}",
        state.text_calls
    );
}
