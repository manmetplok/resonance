//! Host-side tracking of plugin latency changes after activation
//! (doc #260 finding #10, ba todo #1130).
//!
//! The host hands every plugin a `clap_host` vtable whose
//! `get_extension` serves a real `clap_host_latency`; the extension's
//! `changed()` and the host's `request_restart()` both flag the
//! instance so the engine thread cycles its activation
//! (deactivate → reactivate — the only point at which CLAP lets the
//! latency change, and the only point at which the todo-#1125 bridge
//! refreshes its activation-time latency cache) and re-reads
//! `latency.get()`. `reload_with_state` does the same cycle, so a
//! state load that implies a different latency also lands in
//! `latency_samples()`.
//!
//! These tests drive the machinery against a hand-rolled fake CLAP
//! plugin built directly from `clap_sys` vtables — no shared library
//! involved — via the `__instance_from_raw_for_test` hook, which runs
//! the exact `ClapBundle::create_instance` lifecycle sequence.

use std::ffi::{c_char, c_void, CStr};
use std::ptr;

use clap_sys::ext::latency::{clap_host_latency, clap_plugin_latency, CLAP_EXT_LATENCY};
use clap_sys::ext::state::{clap_plugin_state, CLAP_EXT_STATE};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::stream::clap_istream;

use resonance_audio::__test_support::{ClapInstance, __instance_from_raw_for_test};

// ---------------------------------------------------------------------------
// Fake plugin
// ---------------------------------------------------------------------------

/// Backing state for the fake plugin. Mirrors the todo-#1125 bridge
/// semantics: `latency.get()` serves `served_latency`, which is only
/// refreshed from `pending_latency` at activate — so a re-query
/// without an intervening deactivate → reactivate cycle sees the
/// stale value, exactly like the real bridge.
struct FakeState {
    host: *const clap_host,
    /// Latency the plugin will report after its NEXT activation.
    pending_latency: u32,
    /// Latency currently served by `latency.get()` (activation-time
    /// snapshot of `pending_latency`).
    served_latency: u32,
    active: bool,
    activate_calls: u32,
    deactivate_calls: u32,
    load_calls: u32,
}

unsafe fn fake_state<'a>(plugin: *const clap_plugin) -> &'a mut FakeState {
    &mut *((*plugin).plugin_data as *mut FakeState)
}

unsafe extern "C" fn fake_init(_plugin: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn fake_destroy(_plugin: *const clap_plugin) {}

unsafe extern "C" fn fake_activate(
    plugin: *const clap_plugin,
    _sample_rate: f64,
    _min_frames: u32,
    _max_frames: u32,
) -> bool {
    let state = fake_state(plugin);
    state.active = true;
    state.activate_calls += 1;
    // Latency changes take effect at activation only (#1125 bridge).
    state.served_latency = state.pending_latency;
    true
}

unsafe extern "C" fn fake_deactivate(plugin: *const clap_plugin) {
    let state = fake_state(plugin);
    state.active = false;
    state.deactivate_calls += 1;
}

unsafe extern "C" fn fake_start_processing(_plugin: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn fake_stop_processing(_plugin: *const clap_plugin) {}

unsafe extern "C" fn fake_latency_get(plugin: *const clap_plugin) -> u32 {
    fake_state(plugin).served_latency
}

static FAKE_LATENCY_EXT: clap_plugin_latency = clap_plugin_latency {
    get: Some(fake_latency_get),
};

/// State blob = 4 LE bytes: the plugin's new latency. Loading only
/// updates `pending_latency`; the served value moves at reactivation.
unsafe extern "C" fn fake_state_load(
    plugin: *const clap_plugin,
    stream: *const clap_istream,
) -> bool {
    let mut buf = [0u8; 4];
    let read_fn = match (*stream).read {
        Some(f) => f,
        None => return false,
    };
    let n = read_fn(stream, buf.as_mut_ptr() as *mut c_void, 4);
    if n != 4 {
        return false;
    }
    let state = fake_state(plugin);
    state.pending_latency = u32::from_le_bytes(buf);
    state.load_calls += 1;
    true
}

static FAKE_STATE_EXT: clap_plugin_state = clap_plugin_state {
    save: None,
    load: Some(fake_state_load),
};

unsafe extern "C" fn fake_get_extension(
    _plugin: *const clap_plugin,
    id: *const c_char,
) -> *const c_void {
    let id = CStr::from_ptr(id);
    if id.to_bytes() == CLAP_EXT_LATENCY.to_bytes() {
        return &FAKE_LATENCY_EXT as *const clap_plugin_latency as *const c_void;
    }
    if id.to_bytes() == CLAP_EXT_STATE.to_bytes() {
        return &FAKE_STATE_EXT as *const clap_plugin_state as *const c_void;
    }
    ptr::null()
}

/// Build a `ClapInstance` around a fresh fake plugin reporting
/// `initial_latency`, plus a raw pointer to its backing state so tests
/// can inspect call counts and stage the next latency. Both the plugin
/// struct and the state are intentionally leaked — the instance's
/// `Drop` still dereferences them (stop / deactivate / destroy).
fn make_instance(initial_latency: u32) -> (ClapInstance, *mut FakeState) {
    let mut state_ptr: *mut FakeState = ptr::null_mut();
    let instance = __instance_from_raw_for_test(
        |host| {
            let state = Box::into_raw(Box::new(FakeState {
                host,
                pending_latency: initial_latency,
                served_latency: 0,
                active: false,
                activate_calls: 0,
                deactivate_calls: 0,
                load_calls: 0,
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
// Tests
// ---------------------------------------------------------------------------

#[test]
fn initial_latency_queried_after_activation() {
    let (instance, state) = make_instance(128);
    let state = unsafe { &mut *state };
    assert_eq!(state.activate_calls, 1);
    assert!(state.active);
    assert_eq!(instance.latency_samples(), 128);
    // No plugin-initiated callback fired: nothing pending.
    assert!(!instance.take_host_restart_request());
}

#[test]
fn state_load_cycles_activation_and_requeries_latency() {
    let (mut instance, state) = make_instance(100);
    let state = unsafe { &mut *state };
    assert_eq!(instance.latency_samples(), 100);

    // Load a preset that implies 20 000 samples of latency (think: a
    // longer linear-phase FIR). The host must deactivate, load,
    // reactivate, and re-read the latency — a query without the cycle
    // would still see 100 (served_latency only moves at activate).
    let blob = 20_000u32.to_le_bytes();
    assert!(instance.reload_with_state(&blob));

    assert_eq!(state.load_calls, 1);
    assert_eq!(state.deactivate_calls, 1);
    assert_eq!(state.activate_calls, 2);
    assert!(state.active, "plugin left active after reload");
    assert_eq!(
        instance.latency_samples(),
        20_000,
        "state-load path must re-query latency after reactivation"
    );
}

#[test]
fn host_latency_changed_flags_instance_and_restart_requeries() {
    let (mut instance, state) = make_instance(64);
    let state = unsafe { &mut *state };

    // The plugin asks the host for clap_host_latency — must be served
    // (this returned null before finding #10 was fixed).
    let host = state.host;
    let get_ext = unsafe { (*host).get_extension }.expect("host get_extension");
    let ext = unsafe { get_ext(host, CLAP_EXT_LATENCY.as_ptr()) };
    assert!(
        !ext.is_null(),
        "host must serve the clap_host_latency extension"
    );

    // Plugin flow: stage a new latency, then signal changed().
    state.pending_latency = 512;
    let latency_host_ext = unsafe { &*(ext as *const clap_host_latency) };
    unsafe { latency_host_ext.changed.expect("changed fn")(host) };

    // The engine polls the flag once...
    assert!(instance.take_host_restart_request());
    // ...and it is consumed.
    assert!(!instance.take_host_restart_request());

    // Acting on it = deactivate → reactivate → re-query.
    assert!(instance.restart());
    assert_eq!(state.deactivate_calls, 1);
    assert_eq!(state.activate_calls, 2);
    assert!(state.active);
    assert_eq!(instance.latency_samples(), 512);
    // The cycle leaves no stale request behind.
    assert!(!instance.take_host_restart_request());
}

#[test]
fn host_request_restart_flags_instance() {
    let (instance, state) = make_instance(0);
    let state = unsafe { &mut *state };

    let host = state.host;
    let request_restart = unsafe { (*host).request_restart }.expect("request_restart fn");
    unsafe { request_restart(host) };

    assert!(instance.take_host_restart_request());
    assert!(!instance.take_host_restart_request());
}

#[test]
fn host_get_extension_returns_null_for_unknown_ids() {
    let (_instance, state) = make_instance(0);
    let state = unsafe { &mut *state };
    let host = state.host;
    let get_ext = unsafe { (*host).get_extension }.expect("host get_extension");
    // The host implements no other extension — e.g. clap.state's host
    // side stays unserved.
    let ext = unsafe { get_ext(host, CLAP_EXT_STATE.as_ptr()) };
    assert!(ext.is_null());
}
