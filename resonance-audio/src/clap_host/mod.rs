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
//! - [`state`]: CLAP state extension (save / load / reload / reset).
//! - [`gui`]: CLAP GUI extension (open / close the editor window).
//! - host callbacks (in this file): the no-op `clap_host` vtable we
//!   hand back to plugins. We never need the host-side calls.

mod bundle;
mod gui;
mod instance;
mod process;
mod state;

pub use bundle::ClapBundle;
pub use instance::{ClapInstance, StereoBufMut};

use std::ffi::{c_char, c_void};
use std::pin::Pin;
use std::ptr;

use clap_sys::host::clap_host;
use clap_sys::version::CLAP_VERSION;

// ---------------------------------------------------------------------------
// Host callbacks (minimal no-op implementation)
// ---------------------------------------------------------------------------

pub(super) struct HostData {
    pub clap_host: clap_host,
}

unsafe extern "C" fn host_get_extension(
    _host: *const clap_host,
    _extension_id: *const c_char,
) -> *const c_void {
    ptr::null()
}

unsafe extern "C" fn host_request_restart(_host: *const clap_host) {}
unsafe extern "C" fn host_request_process(_host: *const clap_host) {}
unsafe extern "C" fn host_request_callback(_host: *const clap_host) {}

pub(super) fn create_host_data() -> Pin<Box<HostData>> {
    let mut host_data = Box::pin(HostData {
        clap_host: clap_host {
            clap_version: CLAP_VERSION,
            host_data: ptr::null_mut(),
            name: b"Resonance\0".as_ptr() as *const c_char,
            vendor: b"Resonance\0".as_ptr() as *const c_char,
            url: b"\0".as_ptr() as *const c_char,
            version: b"0.1.0\0".as_ptr() as *const c_char,
            get_extension: Some(host_get_extension),
            request_restart: Some(host_request_restart),
            request_process: Some(host_request_process),
            request_callback: Some(host_request_callback),
        },
    });
    let ptr = &*host_data as *const HostData as *mut c_void;
    unsafe {
        let host_data_mut = Pin::get_unchecked_mut(host_data.as_mut());
        host_data_mut.clap_host.host_data = ptr;
    }
    host_data
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
pub struct SyncClapInstance(pub ClapInstance);

unsafe impl Send for SyncClapInstance {}
unsafe impl Sync for SyncClapInstance {}
