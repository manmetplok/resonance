//! A project save reads what the plugin plays, not what it last reported
//! (code review HOST-01, PUX-01).
//!
//! A plugin's editor writes its params directly. A Resonance plugin keeps a
//! mirror of them for the main thread (`get_value`, `state.save`) and
//! refreshes it only at a `process()` or a `params.flush` — and with the
//! transport stopped no `process()` runs. So `SaveAllPluginStates` flushes
//! each instance first, then reports the values that moved since the host
//! last read them (ahead of the states, so the app's mirror — which the
//! project file's param list is written from — carries them), then saves.
//!
//! The fake plugin here behaves like the bridge: `live` is what its editor
//! wrote, `mirror` is what `get_value` and `state.save` serve, and only
//! `params.flush` (or `process`) copies one into the other.

use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::Arc;

use clap_sys::events::{clap_input_events, clap_output_events};
use clap_sys::ext::params::{clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS};
use clap_sys::ext::state::{clap_plugin_state, CLAP_EXT_STATE};
use clap_sys::id::clap_id;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::clap_process;
use clap_sys::stream::{clap_istream, clap_ostream};

use resonance_audio::test_support::{
    EngineHandlerHarness, PluginSlot, __instance_from_raw_for_test,
};
use resonance_audio::types::{AudioCommand, AudioEvent, PluginInstanceId};

const PARAM: clap_id = 7;
const INSTANCE: PluginInstanceId = 41;

#[derive(Default)]
struct FakeState {
    /// What the editor wrote: what the plugin plays.
    live: f64,
    /// What the plugin reports to the host.
    mirror: f64,
    flushes: u32,
}

unsafe fn fake_state<'a>(plugin: *const clap_plugin) -> &'a mut FakeState {
    unsafe { &mut *((*plugin).plugin_data as *mut FakeState) }
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
unsafe extern "C" fn fake_process(
    plugin: *const clap_plugin,
    _process: *const clap_process,
) -> i32 {
    let state = unsafe { fake_state(plugin) };
    state.mirror = state.live;
    0
}

unsafe extern "C" fn params_count(_plugin: *const clap_plugin) -> u32 {
    1
}
unsafe extern "C" fn params_get_info(
    _plugin: *const clap_plugin,
    index: u32,
    info: *mut clap_param_info,
) -> bool {
    if index != 0 {
        return false;
    }
    let mut name = [0 as c_char; clap_sys::string_sizes::CLAP_NAME_SIZE];
    for (dst, src) in name.iter_mut().zip(b"Mix\0") {
        *dst = *src as c_char;
    }
    unsafe {
        info.write(clap_param_info {
            id: PARAM,
            flags: 0,
            cookie: ptr::null_mut(),
            name,
            module: [0; clap_sys::string_sizes::CLAP_PATH_SIZE],
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.5,
        });
    }
    true
}
unsafe extern "C" fn params_get_value(
    plugin: *const clap_plugin,
    id: clap_id,
    value: *mut f64,
) -> bool {
    if id != PARAM {
        return false;
    }
    unsafe { *value = fake_state(plugin).mirror };
    true
}
unsafe extern "C" fn params_flush(
    plugin: *const clap_plugin,
    _in: *const clap_input_events,
    _out: *const clap_output_events,
) {
    let state = unsafe { fake_state(plugin) };
    state.mirror = state.live;
    state.flushes += 1;
}

static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(params_count),
    get_info: Some(params_get_info),
    get_value: Some(params_get_value),
    value_to_text: None,
    text_to_value: None,
    flush: Some(params_flush),
};

/// The state is the mirrored value, as 8 little-endian bytes.
unsafe extern "C" fn state_save(plugin: *const clap_plugin, stream: *const clap_ostream) -> bool {
    let bytes = unsafe { fake_state(plugin) }.mirror.to_le_bytes();
    unsafe {
        let write = (*stream).write.expect("ostream write");
        write(stream, bytes.as_ptr() as *const c_void, bytes.len() as u64) == bytes.len() as i64
    }
}
unsafe extern "C" fn state_load(_plugin: *const clap_plugin, _stream: *const clap_istream) -> bool {
    true
}

static STATE: clap_plugin_state = clap_plugin_state {
    save: Some(state_save),
    load: Some(state_load),
};

unsafe extern "C" fn get_extension(
    _plugin: *const clap_plugin,
    id: *const c_char,
) -> *const c_void {
    let id = unsafe { CStr::from_ptr(id) }.to_bytes();
    if id == CLAP_EXT_PARAMS.to_bytes() {
        return &PARAMS as *const clap_plugin_params as *const c_void;
    }
    if id == CLAP_EXT_STATE.to_bytes() {
        return &STATE as *const clap_plugin_state as *const c_void;
    }
    ptr::null()
}

/// A harness holding one fake instance whose host has read its params
/// once (as `PluginAdded` does), plus its leaked backing state.
fn harness_with_fake(start: f64) -> (EngineHandlerHarness, &'static mut FakeState) {
    let state: &'static mut FakeState = Box::leak(Box::new(FakeState {
        live: start,
        mirror: start,
        flushes: 0,
    }));
    let data = state as *mut FakeState as *mut c_void;
    let instance = __instance_from_raw_for_test(
        |_host| {
            Box::into_raw(Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: data,
                init: Some(fake_init),
                destroy: Some(fake_destroy),
                activate: Some(fake_activate),
                deactivate: Some(fake_deactivate),
                start_processing: Some(fake_start_processing),
                stop_processing: Some(fake_stop_processing),
                reset: None,
                process: Some(fake_process),
                get_extension: Some(get_extension),
                on_main_thread: None,
            })) as *const clap_plugin
        },
        48_000,
    )
    .expect("fake plugin instance");
    let _ = instance.query_params();
    let mut harness = EngineHandlerHarness::new();
    harness.shared().edit_plugins(|plugins| {
        plugins.insert(INSTANCE, Arc::new(PluginSlot::new(instance)));
    });
    let _ = harness.drain_events();
    (harness, unsafe { &mut *(data as *mut FakeState) })
}

fn saved_value(states: &[(PluginInstanceId, Vec<u8>)]) -> f64 {
    let (_, blob) = states
        .iter()
        .find(|(id, _)| *id == INSTANCE)
        .expect("the instance's state was saved");
    f64::from_le_bytes(blob.as_slice().try_into().expect("8 bytes"))
}

#[test]
fn a_project_save_carries_an_editor_edit_made_with_the_transport_stopped() {
    let (mut harness, plugin) = harness_with_fake(0.25);
    // The editor moves the knob; no block runs.
    plugin.live = 0.75;

    harness.dispatch(AudioCommand::SaveAllPluginStates);
    let events = harness.drain_events();

    let values_at = events
        .iter()
        .position(|e| matches!(e, AudioEvent::PluginParamValuesChanged { .. }))
        .expect("the moved value is reported");
    let states_at = events
        .iter()
        .position(|e| matches!(e, AudioEvent::AllPluginStatesSaved { .. }))
        .expect("the states are reported");
    assert!(values_at < states_at, "values first: the app writes its file on the states");
    let AudioEvent::PluginParamValuesChanged { instance_id, values } = &events[values_at] else {
        unreachable!()
    };
    assert_eq!(*instance_id, INSTANCE);
    assert_eq!(values.len(), 1);
    assert_eq!((values[0].id, values[0].value), (PARAM, 0.75));
    let AudioEvent::AllPluginStatesSaved { states } = &events[states_at] else {
        unreachable!()
    };
    assert_eq!(saved_value(states), 0.75, "the saved state has the edit");
}

#[test]
fn a_project_save_reports_no_values_when_nothing_moved() {
    let (mut harness, plugin) = harness_with_fake(0.25);

    harness.dispatch(AudioCommand::SaveAllPluginStates);
    let events = harness.drain_events();

    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AudioEvent::PluginParamValuesChanged { .. })),
        "{events:?}"
    );
    assert!(plugin.flushes >= 1, "the save still flushed");
}

#[test]
fn a_single_state_save_flushes_first() {
    let (mut harness, plugin) = harness_with_fake(0.25);
    plugin.live = 0.5;

    harness.dispatch(AudioCommand::SavePluginState { instance_id: INSTANCE });
    let events = harness.drain_events();

    let saved = events.iter().find_map(|e| match e {
        AudioEvent::PluginStateSaved { instance_id, data } if *instance_id == INSTANCE => {
            Some(f64::from_le_bytes(data.as_slice().try_into().expect("8 bytes")))
        }
        _ => None,
    });
    assert_eq!(saved, Some(0.5));
}
