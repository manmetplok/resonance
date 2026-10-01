//! What a plugin says about a parameter beyond its range and meaning:
//! whether a host may automate it (CLAP `IS_AUTOMATABLE`), whether only
//! the plugin writes it (`IS_READONLY`), and whether its state leaves it
//! out (`com.resonance.param-flags`) — drums-plugin-rework.md §5.1, §5.4.
//!
//! Against a hand-rolled fake built from `clap_sys` vtables, the harness
//! `clap_param_meta.rs` uses.

use std::ffi::{c_char, c_void, CStr};
use std::ptr;

use clap_sys::ext::params::{
    clap_host_params, clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS,
    CLAP_PARAM_IS_AUTOMATABLE, CLAP_PARAM_IS_READONLY, CLAP_PARAM_IS_STEPPED,
    CLAP_PARAM_RESCAN_ALL, CLAP_PARAM_RESCAN_TEXT, CLAP_PARAM_RESCAN_VALUES,
};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;

use resonance_audio::test_support::{ClapInstance, ParamsRefresh, __instance_from_raw_for_test};
use resonance_common::param_flags::{PluginParamFlags, EXTENSION_ID as PARAM_FLAGS};

pub(crate) const P_GAIN: u32 = 1;
pub(crate) const P_KIT: u32 = 2;
pub(crate) const P_PROGRESS: u32 = 3;

pub(crate) struct FakeState {
    pub kit: f64,
    pub progress: f64,
    /// `value_to_text` calls: a values-only refresh must not make any.
    pub text_calls: u32,
    /// `get_info` calls: likewise.
    pub info_calls: u32,
    /// Whether the fake serves `com.resonance.param-flags` (a first-party
    /// plugin does; a third-party one does not).
    pub first_party: bool,
    /// The host vtable the instance was created with, for calling back.
    pub host: *const clap_host,
}

/// Call the host's `clap_host_params.rescan(flags)`, as the plugin would.
pub(crate) unsafe fn plugin_rescans(state: *mut FakeState, flags: u32) {
    let host = (*state).host;
    let ext = ((*host).get_extension.expect("get_extension"))(host, CLAP_EXT_PARAMS.as_ptr())
        as *const clap_host_params;
    ((*ext).rescan.expect("rescan"))(host, flags);
}

unsafe fn fake_state<'a>(plugin: *const clap_plugin) -> &'a mut FakeState {
    &mut *((*plugin).plugin_data as *mut FakeState)
}

unsafe extern "C" fn fake_init(_plugin: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn fake_destroy(_plugin: *const clap_plugin) {}
unsafe extern "C" fn fake_activate(_p: *const clap_plugin, _sr: f64, _min: u32, _max: u32) -> bool {
    true
}
unsafe extern "C" fn fake_deactivate(_plugin: *const clap_plugin) {}
unsafe extern "C" fn fake_start_processing(_plugin: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn fake_stop_processing(_plugin: *const clap_plugin) {}

unsafe extern "C" fn fake_param_count(_plugin: *const clap_plugin) -> u32 {
    3
}

unsafe fn write_cstr(dst: *mut c_char, capacity: usize, s: &str) {
    let bytes = s.as_bytes();
    let n = bytes.len().min(capacity - 1);
    ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, dst, n);
    *dst.add(n) = 0;
}

unsafe extern "C" fn fake_get_info(
    plugin: *const clap_plugin,
    index: u32,
    out: *mut clap_param_info,
) -> bool {
    fake_state(plugin).info_calls += 1;
    let info = &mut *out;
    info.cookie = ptr::null_mut();
    write_cstr(info.module.as_mut_ptr(), info.module.len(), "");
    let (id, name, flags, min, max, default) = match index {
        0 => (P_GAIN, "Gain", CLAP_PARAM_IS_AUTOMATABLE, 0.0, 1.0, 0.5),
        // A selector: settable, not automatable.
        1 => (P_KIT, "Kit", CLAP_PARAM_IS_STEPPED, -1.0, 999.0, -1.0),
        // An output.
        2 => (P_PROGRESS, "Kit Load Progress", CLAP_PARAM_IS_READONLY, 0.0, 1.0, 0.0),
        _ => return false,
    };
    info.id = id;
    write_cstr(info.name.as_mut_ptr(), info.name.len(), name);
    info.flags = flags;
    info.min_value = min;
    info.max_value = max;
    info.default_value = default;
    true
}

unsafe extern "C" fn fake_get_value(plugin: *const clap_plugin, id: u32, out: *mut f64) -> bool {
    let state = fake_state(plugin);
    *out = match id {
        P_GAIN => 0.5,
        P_KIT => state.kit,
        P_PROGRESS => state.progress,
        _ => return false,
    };
    true
}

unsafe extern "C" fn fake_value_to_text(
    plugin: *const clap_plugin,
    id: u32,
    value: f64,
    out: *mut c_char,
    capacity: u32,
) -> bool {
    fake_state(plugin).text_calls += 1;
    let text = match id {
        P_GAIN => format!("{:.0} %", value * 100.0),
        P_KIT => format!("Kit {}", value.round() as i64),
        P_PROGRESS => format!("{:.0} %", value * 100.0),
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

unsafe extern "C" fn fake_is_state_excluded(_plugin: *const c_void, id: u32) -> bool {
    matches!(id, P_KIT | P_PROGRESS)
}

static FAKE_PARAM_FLAGS: PluginParamFlags = PluginParamFlags {
    is_state_excluded: Some(fake_is_state_excluded),
};

unsafe extern "C" fn fake_get_extension(
    plugin: *const clap_plugin,
    id: *const c_char,
) -> *const c_void {
    let id = CStr::from_ptr(id).to_bytes();
    if id == CLAP_EXT_PARAMS.to_bytes() {
        return &FAKE_PARAMS_EXT as *const clap_plugin_params as *const c_void;
    }
    if id == PARAM_FLAGS.to_bytes() && fake_state(plugin).first_party {
        return &FAKE_PARAM_FLAGS as *const PluginParamFlags as *const c_void;
    }
    ptr::null()
}

/// A `ClapInstance` around the fake, plus its backing state. Both are
/// leaked on purpose: `Drop` still dereferences them.
pub(crate) fn make_instance(first_party: bool) -> (ClapInstance, *mut FakeState) {
    let mut state_ptr: *mut FakeState = ptr::null_mut();
    let instance = __instance_from_raw_for_test(
        |host| {
            let state = Box::into_raw(Box::new(FakeState {
                host,
                kit: -1.0,
                progress: 0.0,
                text_calls: 0,
                info_calls: 0,
                first_party,
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

#[test]
fn the_host_reads_automatable_read_only_and_state_excluded() {
    let (instance, _state) = make_instance(true);
    let params = instance.query_params();
    let by_id = |id| params.iter().find(|p| p.id == id).expect("param");

    let gain = by_id(P_GAIN);
    assert!(gain.automatable && !gain.read_only && !gain.state_excluded);
    assert!(gain.host_persisted());

    let kit = by_id(P_KIT);
    assert!(!kit.automatable, "no lane for the kit selector");
    assert!(!kit.read_only, "the selector is settable");
    assert!(kit.state_excluded, "the state carries the kit as a reference");
    assert!(!kit.host_persisted());

    let progress = by_id(P_PROGRESS);
    assert!(progress.read_only && !progress.automatable && progress.state_excluded);
}

/// A plugin without the first-party extension has no state-excluded
/// params — except its read-only outputs, which no host persists.
#[test]
fn a_third_party_plugin_excludes_only_its_outputs() {
    let (instance, _state) = make_instance(false);
    let params = instance.query_params();
    let kit = params.iter().find(|p| p.id == P_KIT).unwrap();
    assert!(!kit.state_excluded && kit.host_persisted());
    let progress = params.iter().find(|p| p.id == P_PROGRESS).unwrap();
    assert!(progress.state_excluded && !progress.host_persisted());
}

/// A values rescan re-reads values, and formats only the params that
/// moved: a plugin reporting a load progress every few percent must not
/// cost a full `query_params` (a `get_info` walk plus a `value_to_text`
/// per choice step) under the instance lock each time (review finding 7).
#[test]
fn a_values_rescan_rereads_only_values_and_formats_only_what_moved() {
    let (mut instance, state) = make_instance(true);
    // What the host learnt at instantiation.
    let _ = instance.query_params();
    let (info_before, text_before) = unsafe { ((*state).info_calls, (*state).text_calls) };

    unsafe {
        (*state).progress = 0.35;
        plugin_rescans(state, CLAP_PARAM_RESCAN_VALUES);
    }
    assert_eq!(
        instance.take_params_refresh(),
        ParamsRefresh::Values { all_text: false }
    );
    let values = instance.refresh_param_values(false);
    assert_eq!(values.len(), 1, "only the moved param: {values:?}");
    assert_eq!(values[0].id, P_PROGRESS);
    assert_eq!(values[0].value, 0.35);
    assert_eq!(values[0].text, "35 %");
    unsafe {
        assert_eq!((*state).info_calls, info_before, "no get_info walk");
        assert_eq!((*state).text_calls, text_before + 1, "one value formatted");
    }

    // Nothing moved: nothing reported.
    unsafe { plugin_rescans(state, CLAP_PARAM_RESCAN_VALUES) };
    let _ = instance.take_params_refresh();
    assert!(instance.refresh_param_values(false).is_empty());
    assert_eq!(instance.take_params_refresh(), ParamsRefresh::None, "consumed");
}

#[test]
fn the_rescan_flags_decide_how_much_is_reread() {
    let (mut instance, state) = make_instance(true);
    unsafe { plugin_rescans(state, CLAP_PARAM_RESCAN_TEXT) };
    assert_eq!(
        instance.take_params_refresh(),
        ParamsRefresh::Values { all_text: true }
    );
    // Before any query the host has no baseline: a text rescan reports
    // every param, each with its text.
    assert_eq!(instance.refresh_param_values(true).len(), 3);

    unsafe { plugin_rescans(state, CLAP_PARAM_RESCAN_VALUES | CLAP_PARAM_RESCAN_ALL) };
    assert_eq!(instance.take_params_refresh(), ParamsRefresh::Full);

    // A load's second look is a full re-read.
    instance.request_params_refresh();
    assert_eq!(instance.take_params_refresh(), ParamsRefresh::Full);
}
