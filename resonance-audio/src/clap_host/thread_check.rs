//! The host side of CLAP `thread-check`, and the per-thread role flag it
//! reports (realtime-multithreading.md §4.6).
//!
//! CLAP's `[audio-thread]` is a *role*, not one OS thread: a host may call
//! a plugin's `process()` from different threads over time, provided the
//! calls never overlap — which the per-instance mutex already guarantees.
//! With the render pool, a track's chain runs on whichever worker claims
//! its job, so a plugin that asserts on thread identity has to be told the
//! truth: each render worker is an audio thread for life
//! ([`mark_audio_thread`]); the live callback and the offline renderers
//! take the role for each render ([`AudioThreadScope`]), as does the host
//! around its own audio-thread calls; `is_audio_thread()` reports it.
//!
//! `is_main_thread()` answers true only on a thread the host itself made a
//! main thread ([`mark_main_thread`]) and only outside an audio-thread role
//! (code review HOST-08). It used to answer "not an audio thread", which
//! told a plugin's own loader or GUI thread that it was the main thread.
//! The host marks:
//! - the engine thread, for life, as it starts;
//! - whichever thread creates an instance — the engine thread for every
//!   live one, the scan or a test's thread otherwise
//!   (`clap_host::create_host_data`);
//! - an offline renderer's thread, which is the main thread between its
//!   blocks (`ClapInstance::set_render_mode`, its first call on every
//!   instance).

use std::cell::Cell;
use std::ffi::c_void;

use clap_sys::ext::thread_check::clap_host_thread_check;
use clap_sys::host::clap_host;

thread_local! {
    static AUDIO_THREAD: Cell<bool> = const { Cell::new(false) };
    static MAIN_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// Declare the calling thread one of the host's main threads for the rest
/// of its life (see the module doc). Never call it on a render worker.
#[inline]
pub(crate) fn mark_main_thread() {
    MAIN_THREAD.with(|flag| flag.set(true));
}

/// Whether the calling thread is a host main thread, outside any
/// audio-thread role — what `clap_host_thread_check.is_main_thread`
/// answers.
#[inline]
pub fn is_main_thread() -> bool {
    !is_audio_thread() && MAIN_THREAD.with(|flag| flag.get())
}

/// Declare the calling thread an audio thread for the rest of its life.
/// Allocation-free after the thread's first call; cheap enough to call
/// every block.
#[inline]
pub(crate) fn mark_audio_thread() {
    AUDIO_THREAD.with(|flag| flag.set(true));
}

/// Take the audio-thread role for a scope, restoring the thread's previous
/// role on drop. For the render entry points on threads that also do other
/// work (the offline render threads, the live callback), and for the
/// host's own `[audio-thread]` calls — `start_processing`,
/// `stop_processing`, `reset`, an active plugin's `params.flush` — made
/// from the engine thread: CLAP defines those as audio-thread calls, and a
/// plugin that checks (u-he Hive does) aborts when told otherwise. The
/// role is legitimate there because the host holds the instance
/// exclusively, so no `process()` can overlap. A TLS swap; allocation-free
/// after the thread's first use.
#[must_use = "the role lasts only as long as the scope guard"]
pub(crate) struct AudioThreadScope(bool);

impl AudioThreadScope {
    #[inline]
    pub(crate) fn enter() -> Self {
        Self(AUDIO_THREAD.with(|flag| flag.replace(true)))
    }
}

impl Drop for AudioThreadScope {
    #[inline]
    fn drop(&mut self) {
        AUDIO_THREAD.with(|flag| flag.set(self.0));
    }
}

/// Whether the calling thread renders audio.
#[inline]
pub fn is_audio_thread() -> bool {
    AUDIO_THREAD.with(|flag| flag.get())
}

unsafe extern "C" fn host_is_main_thread(_host: *const clap_host) -> bool {
    is_main_thread()
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
