//! `clap_plugin_params.flush`: a parameter set while the transport is
//! stopped must reach the plugin.
//!
//! `ClapInstance::set_param` only queues into `pending_params`, and that
//! queue is drained inside `process()`. The mixer skips the arrangement
//! render entirely when the transport is stopped, so before this fix a
//! parameter set in that window never reached the plugin at all — the
//! DSP kept the old value and a `save_state()` taken meanwhile
//! serialised the old value too.
//!
//! `ClapInstance::flush_pending_params` hands the queue to the plugin's
//! `clap_plugin_params.flush` entry point instead. These tests drive it
//! against a hand-rolled fake CLAP plugin built from `clap_sys` vtables
//! (same `__instance_from_raw_for_test` hook as
//! `clap_latency_tracking.rs`) — a real `.clap` shared library is not
//! needed, and the fake records exactly which entry point each value
//! arrived through.

use std::ffi::{c_char, c_void, CStr};
use std::ptr;

use clap_sys::events::{
    clap_event_param_value, clap_input_events, clap_output_events, CLAP_EVENT_PARAM_VALUE,
};
use clap_sys::ext::params::{clap_plugin_params, CLAP_EXT_PARAMS};
use clap_sys::plugin::clap_plugin;
use clap_sys::process::clap_process;

use resonance_audio::test_support::{__instance_from_raw_for_test, ClapInstance};

// ---------------------------------------------------------------------------
// Fake plugin
// ---------------------------------------------------------------------------

/// Backing state for the fake plugin: every param value it received,
/// tagged with the entry point it arrived through.
#[derive(Default)]
struct FakeState {
    /// `(param_id, value)` pairs delivered via `params.flush`.
    flushed: Vec<(u32, f64)>,
    /// `(param_id, value)` pairs delivered via `process()`'s event list.
    processed: Vec<(u32, f64)>,
    /// Number of `process()` calls, whether or not they carried events.
    process_calls: u32,
}

unsafe fn fake_state<'a>(plugin: *const clap_plugin) -> &'a mut FakeState {
    unsafe { &mut *((*plugin).plugin_data as *mut FakeState) }
}

/// Read every `CLAP_EVENT_PARAM_VALUE` out of a `clap_input_events` list.
unsafe fn drain_param_events(events: *const clap_input_events) -> Vec<(u32, f64)> {
    let mut out = Vec::new();
    if events.is_null() {
        return out;
    }
    unsafe {
        let size_fn = match (*events).size {
            Some(f) => f,
            None => return out,
        };
        let get_fn = match (*events).get {
            Some(f) => f,
            None => return out,
        };
        for i in 0..size_fn(events) {
            let header = get_fn(events, i);
            if header.is_null() || (*header).type_ != CLAP_EVENT_PARAM_VALUE {
                continue;
            }
            let ev = &*(header as *const clap_event_param_value);
            out.push((ev.param_id, ev.value));
        }
    }
    out
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

unsafe extern "C" fn fake_process(plugin: *const clap_plugin, process: *const clap_process) -> i32 {
    unsafe {
        let state = fake_state(plugin);
        state.process_calls += 1;
        state
            .processed
            .extend(drain_param_events((*process).in_events));
    }
    0
}

unsafe extern "C" fn fake_params_flush(
    plugin: *const clap_plugin,
    in_: *const clap_input_events,
    _out: *const clap_output_events,
) {
    unsafe {
        let state = fake_state(plugin);
        state.flushed.extend(drain_param_events(in_));
    }
}

unsafe extern "C" fn fake_params_count(_plugin: *const clap_plugin) -> u32 {
    0
}

/// Params extension with a working `flush` — the normal case.
static PARAMS_WITH_FLUSH: clap_plugin_params = clap_plugin_params {
    count: Some(fake_params_count),
    get_info: None,
    get_value: None,
    value_to_text: None,
    text_to_value: None,
    flush: Some(fake_params_flush),
};

/// Params extension from a plugin that does not implement `flush`
/// (legal: the whole vtable entry is optional).
static PARAMS_WITHOUT_FLUSH: clap_plugin_params = clap_plugin_params {
    count: Some(fake_params_count),
    get_info: None,
    get_value: None,
    value_to_text: None,
    text_to_value: None,
    flush: None,
};

unsafe extern "C" fn get_ext_with_flush(
    _plugin: *const clap_plugin,
    id: *const c_char,
) -> *const c_void {
    if unsafe { CStr::from_ptr(id) }.to_bytes() == CLAP_EXT_PARAMS.to_bytes() {
        return &PARAMS_WITH_FLUSH as *const clap_plugin_params as *const c_void;
    }
    ptr::null()
}

unsafe extern "C" fn get_ext_without_flush(
    _plugin: *const clap_plugin,
    id: *const c_char,
) -> *const c_void {
    if unsafe { CStr::from_ptr(id) }.to_bytes() == CLAP_EXT_PARAMS.to_bytes() {
        return &PARAMS_WITHOUT_FLUSH as *const clap_plugin_params as *const c_void;
    }
    ptr::null()
}

unsafe extern "C" fn get_ext_none(
    _plugin: *const clap_plugin,
    _id: *const c_char,
) -> *const c_void {
    ptr::null()
}

/// Build a `ClapInstance` around a fresh fake plugin whose
/// `get_extension` is `get_ext`, plus a raw pointer to its backing state.
/// Both the plugin struct and the state are intentionally leaked — the
/// instance's `Drop` still dereferences them.
fn make_instance(
    get_ext: unsafe extern "C" fn(*const clap_plugin, *const c_char) -> *const c_void,
) -> (ClapInstance, *mut FakeState) {
    let mut state_ptr: *mut FakeState = ptr::null_mut();
    let instance = __instance_from_raw_for_test(
        |_host| {
            let state = Box::into_raw(Box::new(FakeState::default()));
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
                get_extension: Some(get_ext),
                on_main_thread: None,
            });
            Box::into_raw(plugin) as *const clap_plugin
        },
        48_000,
    )
    .expect("fake plugin instance");
    (instance, state_ptr)
}

/// Run one silent block through the instance, as the mixer would.
fn run_block(instance: &mut ClapInstance) {
    let mut l = [0.0_f32; 64];
    let mut r = [0.0_f32; 64];
    instance.process(&mut l, &mut r, 64);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn flush_delivers_queued_params_without_a_process_call() {
    let (mut instance, state) = make_instance(get_ext_with_flush);
    let state = unsafe { &mut *state };

    // Transport stopped: the mixer never calls process(), so this is the
    // only chance the plugin gets to see the value.
    instance.set_param(7, 0.25);
    instance.set_param(9, 1.0);
    assert!(instance.has_pending_params());

    assert!(instance.flush_pending_params());

    assert_eq!(state.flushed, vec![(7, 0.25), (9, 1.0)]);
    assert_eq!(state.process_calls, 0, "flush must not call process()");
    assert!(
        !instance.has_pending_params(),
        "the queue must be drained by a successful flush"
    );
}

#[test]
fn flushed_params_are_not_replayed_on_the_next_process() {
    let (mut instance, state) = make_instance(get_ext_with_flush);
    let state = unsafe { &mut *state };

    instance.set_param(3, 0.5);
    assert!(instance.flush_pending_params());
    assert_eq!(state.flushed, vec![(3, 0.5)]);

    // Transport starts: the next block must NOT carry the same change
    // again (double-apply would resurrect a value the user has since
    // changed through another path, e.g. the plugin's own editor).
    run_block(&mut instance);
    assert_eq!(state.process_calls, 1);
    assert!(
        state.processed.is_empty(),
        "a flushed param must not be replayed by process(): {:?}",
        state.processed
    );
}

#[test]
fn flush_with_an_empty_queue_does_not_touch_the_plugin() {
    let (mut instance, state) = make_instance(get_ext_with_flush);
    let state = unsafe { &mut *state };

    assert!(!instance.has_pending_params());
    assert!(instance.flush_pending_params());
    assert!(state.flushed.is_empty());
    assert_eq!(state.process_calls, 0);
}

#[test]
fn queue_survives_when_the_plugin_implements_no_flush() {
    let (mut instance, state) = make_instance(get_ext_without_flush);
    let state = unsafe { &mut *state };

    instance.set_param(11, 0.75);
    assert!(
        !instance.flush_pending_params(),
        "no params.flush entry point: the host must report failure"
    );
    assert!(
        instance.has_pending_params(),
        "the queue must stay intact so process() can still apply it"
    );

    run_block(&mut instance);
    assert_eq!(state.processed, vec![(11, 0.75)]);
}

#[test]
fn queue_survives_when_the_plugin_has_no_params_extension() {
    let (mut instance, state) = make_instance(get_ext_none);
    let state = unsafe { &mut *state };

    instance.set_param(2, 0.1);
    assert!(!instance.flush_pending_params());
    assert!(instance.has_pending_params());

    run_block(&mut instance);
    assert_eq!(state.processed, vec![(2, 0.1)]);
}

#[test]
fn repeated_sets_flush_last_value_wins_then_the_queue_is_empty() {
    let (mut instance, state) = make_instance(get_ext_with_flush);
    let state = unsafe { &mut *state };

    // `set_param` dedups by id, so only the final value is delivered.
    instance.set_param(5, 0.1);
    instance.set_param(5, 0.2);
    instance.set_param(5, 0.3);
    assert!(instance.flush_pending_params());
    assert_eq!(state.flushed, vec![(5, 0.3)]);

    // A second flush with nothing queued is a no-op, not a re-send.
    assert!(instance.flush_pending_params());
    assert_eq!(state.flushed, vec![(5, 0.3)]);
}
