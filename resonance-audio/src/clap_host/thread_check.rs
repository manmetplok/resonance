//! The host side of CLAP `thread-check`, and the per-thread role flag it
//! reports (realtime-multithreading.md §4.6).
//!
//! CLAP's `[audio-thread]` is a *role*, not one OS thread: a host may call
//! a plugin's `process()` from different threads over time, provided the
//! calls never overlap — which the per-instance mutex already guarantees.
//! With the render pool, a track's chain runs on whichever worker claims
//! its job, so a plugin that asserts on thread identity has to be told the
//! truth: every thread that renders sets [`mark_audio_thread`] (the live
//! callback, each render worker, the offline render workers), and
//! `is_audio_thread()` reports that flag.
//!
//! `is_main_thread()` answers "not an audio thread". Main-thread calls
//! come from the engine thread in practice, but also from the scan and
//! editor paths; answering `false` on any of them would make a checking
//! plugin refuse a legitimate call, while the only question that matters
//! for the render pool is the audio one.

use std::cell::Cell;
use std::ffi::c_void;

use clap_sys::ext::thread_check::clap_host_thread_check;
use clap_sys::host::clap_host;

thread_local! {
    static AUDIO_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// Declare the calling thread an audio thread for the rest of its life.
/// Allocation-free after the thread's first call; cheap enough to call
/// every block.
#[inline]
pub(crate) fn mark_audio_thread() {
    AUDIO_THREAD.with(|flag| flag.set(true));
}

/// Whether the calling thread renders audio.
#[inline]
pub fn is_audio_thread() -> bool {
    AUDIO_THREAD.with(|flag| flag.get())
}

unsafe extern "C" fn host_is_main_thread(_host: *const clap_host) -> bool {
    !is_audio_thread()
}

unsafe extern "C" fn host_is_audio_thread(_host: *const clap_host) -> bool {
    is_audio_thread()
}

/// The `clap_host_thread_check` vtable. Stateless, so one static serves
/// every instance.
pub(super) static HOST_THREAD_CHECK: clap_host_thread_check = clap_host_thread_check {
    is_main_thread: Some(host_is_main_thread),
    is_audio_thread: Some(host_is_audio_thread),
};

pub(super) fn host_thread_check_ptr() -> *const c_void {
    &HOST_THREAD_CHECK as *const clap_host_thread_check as *const c_void
}
