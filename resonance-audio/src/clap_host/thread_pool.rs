//! The host side of CLAP `thread-pool` (realtime-multithreading.md §5,
//! P4): a plugin inside `process()` may split its work into tasks and ask
//! the host to run them, then continue once all are done.
//!
//! The tasks go to the render pool the calling thread is rendering for
//! (`render_pool::exec_plugin_tasks`): the calling thread runs them too,
//! and idle workers help. Outside a pool — a serial render, or another
//! plugin's batch already using the pool — the calling thread runs them
//! all itself. Either way every task has run when `request_exec` returns
//! `true`, which is all CLAP asks.
//!
//! Plugins known to ask: u-he Hive and MFM2 (their multicore modes).

use std::ffi::c_void;
use std::sync::atomic::Ordering;

use clap_sys::ext::thread_pool::clap_host_thread_pool;
use clap_sys::host::clap_host;

use super::host_data_from;

/// `clap_host_thread_pool.request_exec`. Refused (`false`, so the plugin
/// does the work itself) outside `process()` — CLAP reserves the pool for
/// realtime processing — or before the plugin's `exec` is known.
unsafe extern "C" fn host_request_exec(host: *const clap_host, num_tasks: u32) -> bool {
    // SAFETY: `host` is the pointer this instance was created with.
    let Some(data) = (unsafe { host_data_from(host) }) else {
        return false;
    };
    if !data.in_process.load(Ordering::Acquire) {
        return false;
    }
    let Some(&exec) = data.thread_pool_exec.get() else {
        return false;
    };
    let plugin = data.plugin.load(Ordering::Acquire);
    if plugin.is_null() {
        return false;
    }
    let plugin = PluginPtr(plugin);
    // SAFETY: `exec` is the plugin's own entry point for exactly this, and
    // the plugin is alive — it is inside `process()`, waiting for us.
    crate::render_pool::exec_plugin_tasks(num_tasks, &|task| unsafe { exec(plugin.get(), task) });
    true
}

/// The plugin pointer, handed to helper threads for the length of one
/// `request_exec`: CLAP's `exec` is made to be called from any pool
/// thread while `process()` waits.
#[derive(Clone, Copy)]
struct PluginPtr(*const clap_sys::plugin::clap_plugin);

impl PluginPtr {
    fn get(self) -> *const clap_sys::plugin::clap_plugin {
        self.0
    }
}

// SAFETY: see the type docs; the pointer is only dereferenced by the
// plugin's own `exec`, during the request.
unsafe impl Sync for PluginPtr {}

static HOST_THREAD_POOL: clap_host_thread_pool = clap_host_thread_pool {
    request_exec: Some(host_request_exec),
};

pub(super) fn host_thread_pool_ptr() -> *const c_void {
    &HOST_THREAD_POOL as *const clap_host_thread_pool as *const c_void
}
