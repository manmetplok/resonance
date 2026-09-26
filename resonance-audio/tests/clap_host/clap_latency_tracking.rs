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
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};
use clap_sys::stream::clap_istream;

use resonance_audio::test_support::{
    reload_plugin_state, service_host_restart_request, ClapInstance,
    __instance_from_raw_for_test,
};
use resonance_audio::types::AudioEvent;

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
    /// `process()` calls that reached the plugin.
    process_calls: u32,
    /// When set, the next `activate()` fails (and clears the flag).
    fail_next_activate: bool,
    /// When set, every `activate()` fails.
    fail_all_activates: bool,
    /// `min_frames_count` of the most recent `activate()` call.
    last_min_frames: u32,
    /// When set, the next `start_processing()` fails (and clears it).
    fail_next_start: bool,
    /// True between a successful `start_processing` and `stop_processing`.
    processing: bool,
    /// `process()` calls that arrived while not processing (a CLAP
    /// contract violation by the host).
    process_while_stopped: u32,
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
    min_frames: u32,
    _max_frames: u32,
) -> bool {
    let state = fake_state(plugin);
    state.activate_calls += 1;
    state.last_min_frames = min_frames;
    if std::mem::take(&mut state.fail_next_activate) || state.fail_all_activates {
        return false;
    }
    state.active = true;
    // Latency changes take effect at activation only (#1125 bridge).
    state.served_latency = state.pending_latency;
    true
}

unsafe extern "C" fn fake_deactivate(plugin: *const clap_plugin) {
    let state = fake_state(plugin);
    state.active = false;
    state.deactivate_calls += 1;
}

unsafe extern "C" fn fake_start_processing(plugin: *const clap_plugin) -> bool {
    let state = fake_state(plugin);
    if std::mem::take(&mut state.fail_next_start) {
        return false;
    }
    state.processing = true;
    true
}

unsafe extern "C" fn fake_stop_processing(plugin: *const clap_plugin) {
    fake_state(plugin).processing = false;
}

unsafe extern "C" fn fake_process(
    plugin: *const clap_plugin,
    _process: *const clap_process,
) -> clap_process_status {
    let state = fake_state(plugin);
    state.process_calls += 1;
    if !state.processing {
        state.process_while_stopped += 1;
    }
    CLAP_PROCESS_CONTINUE
}

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
                process_calls: 0,
                fail_next_activate: false,
                fail_all_activates: false,
                last_min_frames: 0,
                fail_next_start: false,
                processing: false,
                process_while_stopped: 0,
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
                process: Some(fake_process),
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

// ---------------------------------------------------------------------------
// Failed state loads (code review ENG-02)
// ---------------------------------------------------------------------------

fn run_block(instance: &mut ClapInstance) {
    let mut l = [0.0f32; 64];
    let mut r = [0.0f32; 64];
    instance.process(&mut l, &mut r, 64);
}

/// A preset the plugin rejects (the fake refuses anything but 4 bytes)
/// used to leave it deactivated for good: silent, no error, and every
/// later load took the `!active` shortcut that never reactivates.
#[test]
fn rejected_state_load_reactivates_with_the_previous_state() {
    let (mut instance, state) = make_instance(100);
    let state = unsafe { &mut *state };

    assert!(!instance.reload_with_state(&[1, 2, 3]), "load must report failure");
    assert!(state.active, "plugin left deactivated after a rejected load");
    assert_eq!(state.activate_calls, 2);
    assert_eq!(instance.latency_samples(), 100, "previous state kept");

    run_block(&mut instance);
    assert_eq!(state.process_calls, 1, "process() must reach the plugin again");

    // A good load afterwards takes the normal path.
    assert!(instance.reload_with_state(&256u32.to_le_bytes()));
    assert!(state.active);
    assert_eq!(instance.latency_samples(), 256);
}

/// When reactivation itself failed, the next load must bring the plugin
/// back instead of loading into a dead instance.
#[test]
fn load_after_failed_reactivation_reactivates() {
    let (mut instance, state) = make_instance(0);
    let state = unsafe { &mut *state };

    state.fail_next_activate = true;
    assert!(!instance.reload_with_state(&64u32.to_le_bytes()));
    assert!(!state.active, "fake refused activation");

    assert!(instance.reload_with_state(&512u32.to_le_bytes()));
    assert!(state.active, "a good load must reactivate a failed instance");
    assert_eq!(instance.latency_samples(), 512);
    run_block(&mut instance);
    assert_eq!(state.process_calls, 1);
}

/// The engine handler's reload surfaces a failure as a user-visible
/// error naming the instance, and stays quiet on success.
#[test]
fn engine_reload_reports_failures_as_errors() {
    let (mut instance, state) = make_instance(0);
    let state = unsafe { &mut *state };

    assert!(reload_plugin_state(&mut instance, 77, &32u32.to_le_bytes()).is_none());

    match reload_plugin_state(&mut instance, 77, &[0xff]) {
        Some(AudioEvent::Error(msg)) => {
            assert!(msg.contains("77"), "error must name the instance: {msg}");
            assert!(msg.contains("previous"), "rejected load keeps the old state: {msg}");
        }
        other => panic!("expected AudioEvent::Error, got {other:?}"),
    }

    state.fail_next_activate = true;
    match reload_plugin_state(&mut instance, 77, &32u32.to_le_bytes()) {
        Some(AudioEvent::Error(msg)) => {
            assert!(msg.contains("77"));
            assert!(msg.contains("silent"), "deactivated plugin must say so: {msg}");
        }
        other => panic!("expected AudioEvent::Error, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Activation block size (code review ENG-10)
// ---------------------------------------------------------------------------

/// The live callback splits a buffer that crosses a loop seam into head
/// and tail sub-blocks of any length, down to 1 frame. The host must not
/// promise the plugin a larger minimum than it actually sends.
#[test]
fn plugins_are_activated_with_a_minimum_block_of_one_frame() {
    let (mut instance, state) = make_instance(0);
    let state = unsafe { &mut *state };
    assert_eq!(state.last_min_frames, 1, "initial activation");
    assert!(instance.restart());
    assert_eq!(state.last_min_frames, 1, "re-activation");
}

// ---------------------------------------------------------------------------
// Failed start_processing (code review ENG-12)
// ---------------------------------------------------------------------------

/// A plugin refusing `start_processing` after `reset_processing`'s stop
/// used to be left activated-but-not-processing while `process()` kept
/// being called on it — which CLAP forbids.
#[test]
fn a_refused_restart_of_processing_never_leads_to_process_on_a_stopped_plugin() {
    let (mut instance, state) = make_instance(0);
    let state = unsafe { &mut *state };

    // Refused once: the reset recovers through a full re-activation.
    state.fail_next_start = true;
    assert!(instance.reset_processing());
    assert!(instance.is_active());
    run_block(&mut instance);
    assert_eq!(state.process_calls, 1);
    assert_eq!(state.process_while_stopped, 0);

    // Refused for good: the instance ends up deactivated and silent.
    state.fail_next_start = true;
    state.fail_all_activates = true;
    assert!(!instance.reset_processing());
    assert!(!instance.is_active());
    run_block(&mut instance);
    assert_eq!(state.process_calls, 1, "process() must not reach a stopped plugin");
    assert_eq!(state.process_while_stopped, 0);
}

// ---------------------------------------------------------------------------
// Servicing restart / latency requests (ENG-12, FU-F2c, FU-M1b)
// ---------------------------------------------------------------------------

fn request_restart(state: &FakeState) {
    let host = state.host;
    unsafe { (*host).request_restart.expect("request_restart fn")(host) };
}

fn signal_latency_changed(state: &FakeState) {
    let host = state.host;
    let get_ext = unsafe { (*host).get_extension }.expect("host get_extension");
    let ext = unsafe { get_ext(host, CLAP_EXT_LATENCY.as_ptr()) } as *const clap_host_latency;
    unsafe { (*ext).changed.expect("changed fn")(host) };
}

/// A plugin that reports a new latency while deactivated (after a failed
/// re-activation, say) is doing what CLAP allows: the latency is read at
/// the next activation. It used to trigger `restart()` on the inactive
/// instance and a spurious "failed to reactivate" error (FU-M1b).
#[test]
fn a_latency_change_on_an_inactive_instance_is_not_an_error() {
    let (mut instance, state) = make_instance(0);
    let state = unsafe { &mut *state };
    state.fail_next_activate = true;
    assert!(!instance.reload_with_state(&64u32.to_le_bytes()));
    assert!(!instance.is_active());
    let activations = state.activate_calls;

    signal_latency_changed(state);
    let (restarted, event) = service_host_restart_request(&mut instance, 5);
    assert!(!restarted);
    assert!(event.is_none(), "no error for a legal latency change: {event:?}");
    assert_eq!(state.activate_calls, activations, "no activation attempted");
}

/// A restart request on an instance a failed re-activation left
/// deactivated retries the activation (FU-F2c), and a failure is
/// reported once, not on every later request (ENG-12).
#[test]
fn a_failed_restart_is_retried_and_reported_once() {
    let (mut instance, state) = make_instance(0);
    let state = unsafe { &mut *state };

    state.fail_all_activates = true;
    request_restart(state);
    let (restarted, event) = service_host_restart_request(&mut instance, 9);
    assert!(!restarted);
    match event {
        Some(AudioEvent::Error(msg)) => assert!(msg.contains('9'), "{msg}"),
        other => panic!("expected AudioEvent::Error, got {other:?}"),
    }
    assert!(!instance.is_active());

    // The plugin keeps asking: retried each time, but reported once.
    let activations = state.activate_calls;
    request_restart(state);
    let (restarted, event) = service_host_restart_request(&mut instance, 9);
    assert!(!restarted);
    assert!(event.is_none(), "the failure was already reported: {event:?}");
    assert_eq!(state.activate_calls, activations + 1, "activation retried");

    // Once it can activate again, the retry brings it back.
    state.fail_all_activates = false;
    request_restart(state);
    let (restarted, event) = service_host_restart_request(&mut instance, 9);
    assert!(restarted);
    assert!(event.is_none());
    assert!(instance.is_active());
    run_block(&mut instance);
    assert_eq!(state.process_calls, 1);

    // A fresh failure after a recovery is news again.
    state.fail_all_activates = true;
    request_restart(state);
    let (_, event) = service_host_restart_request(&mut instance, 9);
    assert!(matches!(event, Some(AudioEvent::Error(_))), "{event:?}");
}

/// No request pending: nothing happens.
#[test]
fn servicing_without_a_request_is_a_no_op() {
    let (mut instance, state) = make_instance(0);
    let state = unsafe { &mut *state };
    let (restarted, event) = service_host_restart_request(&mut instance, 1);
    assert!(!restarted);
    assert!(event.is_none());
    assert_eq!(state.activate_calls, 1);
}
