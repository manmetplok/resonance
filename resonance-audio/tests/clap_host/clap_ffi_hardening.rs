//! FFI hardening at the CLAP host boundary (`clap_host::state`,
//! `clap_host::bundle`).
//!
//! The stream callbacks the host hands to `clap_plugin_state.save/load`
//! are invoked by third-party code over the C ABI, so they must survive
//! a misbehaving plugin: a `write(NULL, 0)` flush idiom, a null buffer
//! with a non-zero size, and an absurd `size` claim that would
//! otherwise drive unbounded `Vec` growth. Per stream.h, errors report
//! -1 (`write`: bytes written or -1; `read`: bytes read, 0 = EOF, -1 =
//! error). These tests drive the real host callbacks end-to-end through
//! a hand-rolled fake CLAP plugin (same harness as
//! `tests/clap_host/plugin_output_scrub.rs` / `tests/clap_host/clap_latency_tracking.rs`)
//! whose `save()`/`load()` misuse the stream on purpose and record what
//! the host returned.
//!
//! Descriptor hardening rides along: CLAP marks only `id` and `name` as
//! mandatory descriptor fields, and a null `vendor` ships in the wild —
//! `descriptor_strings` must map it to the empty string and reject only
//! descriptors missing a mandatory field.

use std::ffi::{c_void, CStr};
use std::ptr;

use clap_sys::ext::state::{clap_plugin_state, CLAP_EXT_STATE};
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};

use resonance_audio::test_support::{ClapBundle, ClapInstance, __instance_from_raw_for_test};

// ---------------------------------------------------------------------------
// Fake plugin
// ---------------------------------------------------------------------------

/// The state payload the well-behaved save script writes; long enough
/// that the chunked load script needs several short reads.
const PAYLOAD: &[u8] = b"resonance fake plugin state";

/// Matches `MAX_STATE_BYTES` in `clap_host/state.rs` — the accumulated
/// state cap the over-cap write test probes one byte past.
const STATE_CAP: u64 = 256 * 1024 * 1024;

/// How the fake plugin's `state.save()` drives the host's ostream.
#[derive(Clone, Copy)]
enum SaveScript {
    /// `write(NULL, 0)` (a flush idiom), then `PAYLOAD` — succeeds.
    NullFlushThenPayload,
    /// `write(NULL, 16)` — a bug the host must refuse, not deref.
    NullNonzero,
    /// Claim `u64::MAX` then `STATE_CAP + 1` bytes against a 4-byte
    /// buffer. The host must refuse both *without* reading the buffer
    /// or allocating; anything else is the alloc-abort this guards.
    AbsurdSizes,
}

/// How the fake plugin's `state.load()` drives the host's istream.
#[derive(Clone, Copy)]
enum LoadScript {
    /// `read(NULL, 8)` — must report -1, not write through null.
    NullRead,
    /// `read(ptr, 0)`, then 8-byte chunks until EOF, collecting data.
    ChunkedReadAll,
}

struct FakeState {
    save_script: SaveScript,
    load_script: LoadScript,
    /// Every stream-callback return value, in call order.
    stream_returns: Vec<i64>,
    /// What `ChunkedReadAll` read back out of the host.
    loaded: Vec<u8>,
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

unsafe extern "C" fn fake_process(
    _plugin: *const clap_plugin,
    _process: *const clap_process,
) -> clap_process_status {
    CLAP_PROCESS_CONTINUE
}

unsafe extern "C" fn fake_save(
    plugin: *const clap_plugin,
    stream: *const clap_sys::stream::clap_ostream,
) -> bool {
    let state = fake_state(plugin);
    let write = (*stream).write.expect("host ostream must have write");
    match state.save_script {
        SaveScript::NullFlushThenPayload => {
            state.stream_returns.push(write(stream, ptr::null(), 0));
            state.stream_returns.push(write(
                stream,
                PAYLOAD.as_ptr() as *const c_void,
                PAYLOAD.len() as u64,
            ));
            true
        }
        SaveScript::NullNonzero => {
            state.stream_returns.push(write(stream, ptr::null(), 16));
            false
        }
        SaveScript::AbsurdSizes => {
            let small = [0u8; 4];
            let small_ptr = small.as_ptr() as *const c_void;
            state.stream_returns.push(write(stream, small_ptr, u64::MAX));
            state
                .stream_returns
                .push(write(stream, small_ptr, STATE_CAP + 1));
            false
        }
    }
}

unsafe extern "C" fn fake_load(
    plugin: *const clap_plugin,
    stream: *const clap_sys::stream::clap_istream,
) -> bool {
    let state = fake_state(plugin);
    let read = (*stream).read.expect("host istream must have read");
    match state.load_script {
        LoadScript::NullRead => {
            state.stream_returns.push(read(stream, ptr::null_mut(), 8));
            false
        }
        LoadScript::ChunkedReadAll => {
            let mut chunk = [0u8; 8];
            let chunk_ptr = chunk.as_mut_ptr() as *mut c_void;
            // A zero-size read is the only case where 0 does not mean
            // EOF; it must still come back as 0, not an error.
            state.stream_returns.push(read(stream, chunk_ptr, 0));
            loop {
                let n = read(stream, chunk_ptr, chunk.len() as u64);
                state.stream_returns.push(n);
                if n <= 0 {
                    return n == 0;
                }
                state.loaded.extend_from_slice(&chunk[..n as usize]);
            }
        }
    }
}

static STATE_EXT: clap_plugin_state = clap_plugin_state {
    save: Some(fake_save),
    load: Some(fake_load),
};

unsafe extern "C" fn fake_get_extension(
    _plugin: *const clap_plugin,
    id: *const std::os::raw::c_char,
) -> *const c_void {
    if CStr::from_ptr(id) == CLAP_EXT_STATE {
        &STATE_EXT as *const clap_plugin_state as *const c_void
    } else {
        ptr::null()
    }
}

/// Build a `ClapInstance` around a fresh fake plugin, plus a raw
/// pointer to its backing state so tests can stage scripts and read
/// back what the host's stream callbacks returned. Both the plugin
/// struct and the state are intentionally leaked — the instance's
/// `Drop` still dereferences them (stop / deactivate / destroy).
fn make_instance(save_script: SaveScript, load_script: LoadScript) -> (ClapInstance, *mut FakeState) {
    let mut state_ptr: *mut FakeState = ptr::null_mut();
    let instance = __instance_from_raw_for_test(
        |_host| {
            let state = Box::into_raw(Box::new(FakeState {
                save_script,
                load_script,
                stream_returns: Vec::new(),
                loaded: Vec::new(),
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
// ostream_write hardening
// ---------------------------------------------------------------------------

#[test]
fn null_flush_write_is_a_no_op_and_payload_still_saves() {
    let (instance, state) =
        make_instance(SaveScript::NullFlushThenPayload, LoadScript::ChunkedReadAll);
    let saved = instance.save_state();
    let state = unsafe { &*state };
    assert_eq!(
        state.stream_returns,
        vec![0, PAYLOAD.len() as i64],
        "write(NULL, 0) must report 0 bytes written; the real payload must land in full"
    );
    assert_eq!(saved.as_deref(), Some(PAYLOAD));
}

#[test]
fn null_buffer_nonzero_write_reports_error() {
    let (instance, state) = make_instance(SaveScript::NullNonzero, LoadScript::ChunkedReadAll);
    let saved = instance.save_state();
    let state = unsafe { &*state };
    assert_eq!(
        state.stream_returns,
        vec![-1],
        "write(NULL, 16) must report -1, never dereference the buffer"
    );
    assert_eq!(saved, None, "the plugin returned false, so no state");
}

#[test]
fn absurd_size_writes_are_rejected_without_allocation() {
    let (instance, state) = make_instance(SaveScript::AbsurdSizes, LoadScript::ChunkedReadAll);
    // If the host trusted either claimed size it would read far past
    // the 4-byte buffer and/or abort in the allocator — reaching the
    // asserts at all is half the test.
    let saved = instance.save_state();
    let state = unsafe { &*state };
    assert_eq!(
        state.stream_returns,
        vec![-1, -1],
        "u64::MAX and cap+1 byte claims must both report -1"
    );
    assert_eq!(saved, None);
}

// ---------------------------------------------------------------------------
// istream_read hardening
// ---------------------------------------------------------------------------

#[test]
fn null_buffer_read_reports_error() {
    let (mut instance, state) =
        make_instance(SaveScript::NullFlushThenPayload, LoadScript::NullRead);
    let ok = instance.load_state(b"some persisted state");
    let state = unsafe { &*state };
    assert_eq!(
        state.stream_returns,
        vec![-1],
        "read(NULL, 8) must report -1, never write through the buffer"
    );
    assert!(!ok, "the plugin returned false, so the load failed");
}

#[test]
fn chunked_reads_deliver_the_full_state_then_eof() {
    let (mut instance, state) =
        make_instance(SaveScript::NullFlushThenPayload, LoadScript::ChunkedReadAll);
    let ok = instance.load_state(PAYLOAD);
    let state = unsafe { &*state };
    assert!(ok);
    assert_eq!(state.loaded, PAYLOAD, "chunked reads must reassemble the state");
    // Zero-size probe, three full/short chunks over the 27-byte
    // payload, then the EOF sentinel.
    assert_eq!(state.stream_returns, vec![0, 8, 8, 8, 3, 0]);
}

// ---------------------------------------------------------------------------
// Descriptor string hardening
// ---------------------------------------------------------------------------

#[test]
fn descriptor_null_vendor_becomes_empty_string() {
    let id = c"com.example.effect";
    let name = c"Example Effect";
    let strings = unsafe {
        ClapBundle::__descriptor_strings_for_test(id.as_ptr(), name.as_ptr(), ptr::null())
    };
    assert_eq!(
        strings,
        Some((
            "com.example.effect".to_string(),
            "Example Effect".to_string(),
            String::new(),
        )),
        "vendor is optional per plugin.h: null must read as empty, not crash"
    );
}

#[test]
fn descriptor_missing_mandatory_field_is_rejected() {
    let id = c"com.example.effect";
    let name = c"Example Effect";
    let vendor = c"Example Vendor";
    let null_name = unsafe {
        ClapBundle::__descriptor_strings_for_test(id.as_ptr(), ptr::null(), vendor.as_ptr())
    };
    assert_eq!(null_name, None, "a null name makes the descriptor unusable");
    let null_id = unsafe {
        ClapBundle::__descriptor_strings_for_test(ptr::null(), name.as_ptr(), vendor.as_ptr())
    };
    assert_eq!(null_id, None, "a null id makes the descriptor unusable");
}

// ---------------------------------------------------------------------------
// Plugin-filled structs (code review HOST-12)
// ---------------------------------------------------------------------------
//
// A second fake whose `params.get_info` and `audio_ports.get` fill their
// fixed-size name arrays to the last byte with no NUL. The host used to
// read them with an unbounded `CStr::from_ptr`, running into the struct's
// next field (and past the struct); it used `MaybeUninit::uninit()` +
// `assume_init` for the param info too, so a plugin that answered `true`
// without writing a field handed the host uninitialised memory.

use clap_sys::ext::audio_ports::{clap_audio_port_info, clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS};
use clap_sys::ext::params::{clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS};
use clap_sys::string_sizes::{CLAP_NAME_SIZE, CLAP_PATH_SIZE};

unsafe extern "C" fn unterminated_param_count(_p: *const clap_plugin) -> u32 {
    2
}

unsafe extern "C" fn unterminated_param_info(
    _p: *const clap_plugin,
    index: u32,
    info: *mut clap_param_info,
) -> bool {
    let info = &mut *info;
    if index == 1 {
        // Says yes, writes nothing: the host must read defined zeros.
        return true;
    }
    info.id = 7;
    info.flags = 0;
    info.min_value = 0.0;
    info.max_value = 1.0;
    info.default_value = 0.5;
    info.name.fill(b'N' as std::os::raw::c_char);
    info.module.fill(b'M' as std::os::raw::c_char);
    true
}

static UNTERMINATED_PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(unterminated_param_count),
    get_info: Some(unterminated_param_info),
    get_value: None,
    value_to_text: None,
    text_to_value: None,
    flush: None,
};

unsafe extern "C" fn one_port(_p: *const clap_plugin, _is_input: bool) -> u32 {
    1
}

unsafe extern "C" fn unterminated_port(
    _p: *const clap_plugin,
    _index: u32,
    _is_input: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    let info = &mut *info;
    info.id = 0;
    info.channel_count = 2;
    info.name.fill(b'P' as std::os::raw::c_char);
    true
}

static UNTERMINATED_PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(one_port),
    get: Some(unterminated_port),
};

unsafe extern "C" fn unterminated_get_extension(
    _plugin: *const clap_plugin,
    id: *const std::os::raw::c_char,
) -> *const c_void {
    let id = CStr::from_ptr(id);
    if id == CLAP_EXT_PARAMS {
        &UNTERMINATED_PARAMS as *const clap_plugin_params as *const c_void
    } else if id == CLAP_EXT_AUDIO_PORTS {
        &UNTERMINATED_PORTS as *const clap_plugin_audio_ports as *const c_void
    } else {
        ptr::null()
    }
}

fn unterminated_instance() -> ClapInstance {
    __instance_from_raw_for_test(
        |_host| {
            let state = Box::into_raw(Box::new(FakeState {
                save_script: SaveScript::NullFlushThenPayload,
                load_script: LoadScript::ChunkedReadAll,
                stream_returns: Vec::new(),
                loaded: Vec::new(),
            }));
            Box::into_raw(Box::new(clap_plugin {
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
                get_extension: Some(unterminated_get_extension),
                on_main_thread: None,
            })) as *const clap_plugin
        },
        48_000,
    )
    .expect("fake plugin instance")
}

#[test]
fn an_unterminated_param_name_is_read_no_further_than_its_array() {
    let instance = unterminated_instance();
    let params = instance.query_params();
    assert_eq!(params.len(), 2);
    let p = &params[0];
    assert_eq!(p.id, 7);
    assert_eq!(p.name, "N".repeat(CLAP_NAME_SIZE), "name read past its array");
    assert_eq!(p.module, "M".repeat(CLAP_PATH_SIZE), "module read past its array");
    // The info the plugin claimed but never wrote reads as zeros.
    let blank = &params[1];
    assert_eq!((blank.id, blank.name.as_str(), blank.max_value), (0, "", 0.0));
}

#[test]
fn an_unterminated_port_name_is_read_no_further_than_its_array() {
    let instance = unterminated_instance();
    assert_eq!(instance.output_port_names(), vec!["P".repeat(CLAP_NAME_SIZE)]);
}

/// A caller handing `process()` buffers shorter than `frames` is a bug
/// upstream; debug builds catch it, release builds cut the block short
/// instead of letting the plugin run off the slice (HOST-12).
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "frames against buffers of")]
fn process_with_frames_past_the_buffers_is_caught_in_debug() {
    let (mut instance, _state) =
        make_instance(SaveScript::NullFlushThenPayload, LoadScript::ChunkedReadAll);
    let mut l = vec![0.0f32; 64];
    let mut r = vec![0.0f32; 64];
    instance.process(&mut l, &mut r, 128);
}
