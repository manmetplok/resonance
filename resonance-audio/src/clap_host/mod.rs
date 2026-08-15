//! Minimal CLAP plugin host. Loads `.clap` shared libraries, instantiates
//! plugins, and runs them through the audio callback.
//!
//! Submodules by concern:
//! - [`bundle`]: load a `.clap` library, walk its plugin factory.
//! - [`instance`]: per-plugin lifecycle, parameter / note queues,
//!   transport latching, the [`StereoBufMut`] borrow type used to
//!   pass per-port output slices into the audio thread.
//! - [`process`]: the audio-thread fast path ([`ClapInstance::process`]
//!   single-output wrapper and [`ClapInstance::process_multi`]).
//! - [`param_meta`]: what a parameter *means* — its unit, its choice
//!   labels — read back out of the plugin's own formatter.
//! - [`params`]: `clap_plugin_params.flush` — delivers queued parameter
//!   changes when no `process()` call is coming (transport stopped).
//! - [`state`]: CLAP state extension (save / load / reload / reset).
//! - [`gui`]: CLAP GUI extension (open / close the editor window).
//! - host callbacks (in this file): the `clap_host` vtable we hand back
//!   to plugins. Mostly no-ops, with two exceptions serviced by the
//!   engine thread (doc #260 finding #10): `clap_host_latency.changed()`
//!   and `request_restart()` both flag the instance so the engine
//!   deactivates → reactivates it at the next safe point and re-reads
//!   its latency.

mod bundle;
mod gui;
mod instance;
mod param_meta;
mod params;
mod process;
mod state;

pub use bundle::ClapBundle;
pub use instance::{ClapInstance, StereoBufMut};
pub use param_meta::{choice_labels, unit_from_text, MAX_CHOICE_STEPS};

use std::ffi::{c_char, c_void, CStr};
use std::pin::Pin;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use clap_sys::ext::latency::{clap_host_latency, CLAP_EXT_LATENCY};
use clap_sys::host::clap_host;
use clap_sys::version::CLAP_VERSION;

// ---------------------------------------------------------------------------
// Host callbacks
// ---------------------------------------------------------------------------

pub(super) struct HostData {
    pub clap_host: clap_host,
    /// `clap_host_latency` vtable served from `host_get_extension`.
    /// Lives inside the pinned `HostData` so the pointer we hand the
    /// plugin stays stable for the instance's lifetime.
    pub latency_ext: clap_host_latency,
    /// Set by `clap_host_latency.changed()`: the plugin's reported
    /// latency is out of date. Because the bridge serves an
    /// activation-time cached value while active (todo #1125), acting
    /// on this means a deactivate → reactivate cycle, same as a
    /// restart request. Consumed by
    /// [`ClapInstance::take_host_restart_request`].
    pub latency_changed: AtomicBool,
    /// Set by `clap_host.request_restart()`: the plugin asks for a
    /// deactivate → reactivate cycle. Consumed by
    /// [`ClapInstance::take_host_restart_request`].
    pub restart_requested: AtomicBool,
}

/// Recover the `HostData` behind a `clap_host` pointer handed back by a
/// plugin. Returns `None` for null / not-yet-wired pointers so a
/// misbehaving plugin calling into us mid-construction can't crash.
unsafe fn host_data_from<'a>(host: *const clap_host) -> Option<&'a HostData> {
    if host.is_null() {
        return None;
    }
    let data = (*host).host_data as *const HostData;
    data.as_ref()
}

unsafe extern "C" fn host_get_extension(
    host: *const clap_host,
    extension_id: *const c_char,
) -> *const c_void {
    let Some(data) = host_data_from(host) else {
        return ptr::null();
    };
    if extension_id.is_null() {
        return ptr::null();
    }
    if CStr::from_ptr(extension_id).to_bytes() == CLAP_EXT_LATENCY.to_bytes() {
        return &data.latency_ext as *const clap_host_latency as *const c_void;
    }
    ptr::null()
}

/// `clap_host_latency.changed()` — the plugin's latency is stale.
/// Callable per spec on the main thread (incl. during activation); we
/// only flip an atomic flag here, the actual reactivate + re-query runs
/// on the engine thread at the next loop iteration.
unsafe extern "C" fn host_latency_changed(host: *const clap_host) {
    if let Some(data) = host_data_from(host) {
        data.latency_changed.store(true, Ordering::Release);
    }
}

/// `clap_host.request_restart()` — schedule a deactivate → reactivate
/// cycle. Flag only; serviced by the engine thread.
unsafe extern "C" fn host_request_restart(host: *const clap_host) {
    if let Some(data) = host_data_from(host) {
        data.restart_requested.store(true, Ordering::Release);
    }
}

unsafe extern "C" fn host_request_process(_host: *const clap_host) {}
unsafe extern "C" fn host_request_callback(_host: *const clap_host) {}

pub(super) fn create_host_data() -> Pin<Box<HostData>> {
    let mut host_data = Box::pin(HostData {
        clap_host: clap_host {
            clap_version: CLAP_VERSION,
            host_data: ptr::null_mut(),
            name: c"Resonance".as_ptr(),
            vendor: c"Resonance".as_ptr(),
            url: c"".as_ptr(),
            version: c"0.1.0".as_ptr(),
            get_extension: Some(host_get_extension),
            request_restart: Some(host_request_restart),
            request_process: Some(host_request_process),
            request_callback: Some(host_request_callback),
        },
        latency_ext: clap_host_latency {
            changed: Some(host_latency_changed),
        },
        latency_changed: AtomicBool::new(false),
        restart_requested: AtomicBool::new(false),
    });
    let ptr = &*host_data as *const HostData as *mut c_void;
    unsafe {
        let host_data_mut = Pin::get_unchecked_mut(host_data.as_mut());
        host_data_mut.clap_host.host_data = ptr;
    }
    host_data
}

/// Test-only: build a [`ClapInstance`] around a raw `clap_plugin`
/// produced by `build` (which receives the host vtable pointer exactly
/// like a factory's `create_plugin` would). Runs the same
/// init / extension-query / activate / latency-query / start sequence
/// as [`ClapBundle::create_instance`], so integration tests can drive
/// the host-side latency machinery against a hand-rolled fake plugin
/// without a shared library.
#[doc(hidden)]
pub fn __instance_from_raw_for_test(
    build: impl FnOnce(*const clap_host) -> *const clap_sys::plugin::clap_plugin,
    sample_rate: u32,
) -> Result<ClapInstance, String> {
    let host_data = create_host_data();
    let host_ptr = &host_data.clap_host as *const clap_host;
    let plugin = build(host_ptr);
    if plugin.is_null() {
        return Err("test build fn returned a null plugin".to_string());
    }
    bundle::build_instance(plugin, host_data, sample_rate)
}

// ---------------------------------------------------------------------------
// SyncClapInstance — Send + Sync wrapper
// ---------------------------------------------------------------------------

/// Wrapper that makes [`ClapInstance`] `Send + Sync`.
///
/// SAFETY: This is justified by the CLAP threading contract:
/// - Lifecycle methods (create/activate/destroy) are called from the engine thread only
/// - process() is called from the audio callback thread only
/// - set_param() is called from the engine thread, pending_params consumed by process()
/// - flush_pending_params() (`clap_plugin_params.flush`) may only run
///   while no process() call is in flight for the same instance. The
///   mutex around this wrapper is what enforces that: `&mut ClapInstance`
///   is unreachable without its guard, and every process() call site
///   takes the same lock. See `clap_host::params` for the full argument.
pub struct SyncClapInstance(pub ClapInstance);

unsafe impl Send for SyncClapInstance {}
unsafe impl Sync for SyncClapInstance {}
