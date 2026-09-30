//! The plugin-facing handle to the CLAP host.
//!
//! A [`ResonancePlugin`](crate::plugin::ResonancePlugin) receives an
//! `Arc<HostHandle>` from the CLAP bridge through
//! [`ResonancePlugin::set_host`](crate::plugin::ResonancePlugin::set_host),
//! once, on the main thread, right after construction. It is the only way a
//! plugin can talk *back* to the host.
//!
//! Today it carries the latency channel (ba todo #1296). Latency is the one
//! piece of plugin state a host cannot poll for: CLAP only defines
//! `clap_plugin_latency.get()` while the plugin is active, and at that point
//! the bridge has moved the plugin object into the audio processor where the
//! main thread cannot reach it. So a plugin whose latency changes at runtime
//! — a limiter lookahead knob, an IR block-size switch, an oversampling
//! selector — *must* push, or every other track in the project stays
//! compensated for the old figure while this one delays by the new one.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use clack_plugin::prelude::HostSharedHandle;

/// Top bit of `HostHandle::calls`: the handle has been retired.
const RETIRED: usize = 1 << (usize::BITS - 1);

/// Handle to the host that owns this plugin instance.
///
/// Cheap to clone (it is always handed out behind an `Arc`) and safe to call
/// from any thread — every operation on it is either an atomic or one of the
/// three `clap_host` callbacks the CLAP spec marks `[thread-safe]`
/// (`request_restart`, `request_process`, `request_callback`). None of them
/// allocates or locks, so the audio thread may use them.
///
/// A handle whose plugin instance has been destroyed goes inert: every method
/// becomes a no-op instead of calling through a dangling host pointer, and
/// the instance's destruction waits for any call already inside the host.
/// That makes it safe for a plugin to leak a clone into an editor thread
/// that outlives the instance.
pub struct HostHandle {
    /// The host's `clap_host` handle with its lifetime erased.
    ///
    /// SAFETY: the real lifetime is the plugin instance's `'a`. It is upheld
    /// by `calls`: the bridge retires the handle in `ClapMainThread::drop`,
    /// i.e. while the instance — and therefore the host pointer — is still
    /// valid, and `retire` does not return until every call already
    /// through the check has left the host. See [`Self::with_host`] and
    /// `clap_bridge::shared::ClapMainThread`.
    host: HostSharedHandle<'static>,
    /// [`RETIRED`] once the plugin instance is being destroyed, plus, in
    /// the remaining bits, the number of calls into the host in flight
    /// right now (PLG-07).
    calls: AtomicUsize,
    /// True while the plugin is activated. Governs whether a latency change
    /// also needs a restart request (CLAP only allows the reported latency to
    /// change while the plugin is deactivated).
    active: AtomicBool,
    /// The latency the plugin last reported, in samples. Serves the CLAP
    /// `latency.get()` query while the plugin lives in the audio processor.
    latency: AtomicU32,
    /// Set when `latency` changed and the host has not been told yet.
    /// Consumed on the main thread, which is the only thread allowed to call
    /// `clap_host_latency.changed()`.
    latency_dirty: AtomicBool,
    /// The serial of an editor whose window closed itself and the host has
    /// not been told yet; 0 when there is none. Set from whatever thread
    /// the GUI runtime closes on, consumed on the main thread, which is
    /// the only one allowed to call `clap_host_gui.closed()` (PLG-01).
    /// A serial rather than a flag, so a late report from an editor the
    /// host has since destroyed cannot be pinned on its successor.
    gui_closed: AtomicU64,
    /// The loaded preset identity or its modified flag changed and the
    /// host has not been told. Set from any thread (the session's
    /// notifier), consumed on the main thread.
    preset_dirty: AtomicBool,
}

impl HostHandle {
    /// Wrap the bridge's host handle. Called once per plugin instance, on the
    /// main thread, before the plugin can ever be activated.
    pub(crate) fn new(host: HostSharedHandle<'_>, initial_latency: u32) -> Arc<Self> {
        // SAFETY: the erased lifetime is re-established by `calls` — see the
        // field docs. This handle is created inside `new_main_thread`, where
        // the host pointer is live by construction.
        let host: HostSharedHandle<'static> = unsafe { host.with_arbitrary_lifetime() };
        Arc::new(Self {
            host,
            calls: AtomicUsize::new(0),
            active: AtomicBool::new(false),
            latency: AtomicU32::new(initial_latency),
            latency_dirty: AtomicBool::new(false),
            gui_closed: AtomicU64::new(0),
            preset_dirty: AtomicBool::new(false),
        })
    }

    /// Report a new processing latency, in samples.
    ///
    /// Call this whenever the plugin's latency changes after activation —
    /// from the audio thread inside `process()`, or from an editor thread.
    /// It is realtime-safe: two relaxed atomics plus, only when the value
    /// actually changed, the host's `[thread-safe]` request callbacks.
    ///
    /// The plugin's own [`latency_samples()`](crate::plugin::ResonancePlugin::latency_samples)
    /// must already agree with `samples` by the time this returns — the host
    /// re-queries it through the CLAP latency extension, and while the plugin
    /// is *inactive* that query goes straight to the plugin object.
    ///
    /// What the bridge does with it, per the CLAP latency extension: the
    /// reported value is only allowed to change while the plugin is
    /// deactivated, so if the plugin is currently active the bridge asks the
    /// host for a restart (deactivate → reactivate) and, on the next main
    /// thread callback, calls `clap_host_latency.changed()` so the host
    /// re-reads the latency and recomputes plugin delay compensation.
    /// Redundant reports (same value) do nothing at all.
    pub fn set_latency_samples(&self, samples: u32) {
        if self.latency.swap(samples, Ordering::AcqRel) == samples {
            return;
        }
        self.latency_dirty.store(true, Ordering::Release);

        // The reported latency may only change while deactivated; ask for the
        // deactivate → reactivate cycle when we are mid-flight.
        if self.active.load(Ordering::Acquire) {
            self.request_restart();
        }
        // `clap_host_latency.changed()` is [main-thread]; we may be on the
        // audio thread. Ask the host to call us back there — the bridge's
        // `on_main_thread` drains `latency_dirty`.
        self.request_callback();
    }

    /// The latency the plugin last reported, in samples.
    pub fn latency_samples(&self) -> u32 {
        self.latency.load(Ordering::Acquire)
    }

    /// Ask the host to deactivate and then reactivate the plugin. Delayed to
    /// a safe point by the host; never synchronous.
    pub fn request_restart(&self) {
        self.with_host(|host| host.request_restart());
    }

    /// Ask the host to activate the plugin and start processing.
    pub fn request_process(&self) {
        self.with_host(|host| host.request_process());
    }

    /// Ask the host to call the plugin back on the main thread.
    pub fn request_callback(&self) {
        self.with_host(|host| host.request_callback());
    }

    /// Call through the host pointer unless the handle is retired.
    ///
    /// The check and the call are one unit with respect to [`Self::retire`]
    /// (PLG-07): the call is counted in `calls` *before* the retired bit is
    /// looked at, and `retire` sets the bit and then waits for the count to
    /// drain. So a call either sees the bit and does nothing, or is counted
    /// and finishes before `retire` returns — before the host can free the
    /// pointer. Two uncontended atomic RMWs; no lock, no allocation, so the
    /// audio thread may still use it.
    fn with_host(&self, call: impl FnOnce(&HostSharedHandle<'static>)) {
        let prev = self.calls.fetch_add(1, Ordering::AcqRel);
        if prev & RETIRED == 0 {
            call(&self.host);
        }
        self.calls.fetch_sub(1, Ordering::Release);
    }

    // -- bridge-internal ----------------------------------------------------

    /// Record the latency the bridge read straight from the plugin (at
    /// activation, or on an inactive host query). Does not notify the host:
    /// the host asked, or is about to ask, for this value itself.
    pub(crate) fn store_latency(&self, samples: u32) {
        self.latency.store(samples, Ordering::Release);
    }

    /// Take the "host has not been told about the new latency" flag.
    pub(crate) fn take_latency_dirty(&self) -> bool {
        self.latency_dirty.swap(false, Ordering::AcqRel)
    }

    /// Track the plugin's activation state, so `set_latency_samples` knows
    /// whether a restart request is required.
    pub(crate) fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::Release);
    }

    /// The editor with this serial closed its own window. Latch it and
    /// ask for a main-thread callback, where the bridge's `on_main_thread`
    /// turns it into `clap_host_gui.closed()` (a `[main-thread]` call; the
    /// runtime may be on its own thread).
    pub(crate) fn report_gui_closed(&self, editor_serial: u64) {
        self.gui_closed.store(editor_serial, Ordering::Release);
        self.request_callback();
    }

    /// The preset identity or its modified flag changed: latch it and ask
    /// for a main-thread callback, where the bridge reports it.
    pub(crate) fn report_preset_change(&self) {
        if !self.preset_dirty.swap(true, Ordering::AcqRel) {
            self.request_callback();
        }
    }

    /// Take the "preset identity changed" flag.
    pub(crate) fn take_preset_dirty(&self) -> bool {
        self.preset_dirty.swap(false, Ordering::AcqRel)
    }

    /// Take the pending self-close report: the serial of the editor that
    /// closed, or `None`.
    pub(crate) fn take_gui_closed(&self) -> Option<u64> {
        match self.gui_closed.swap(0, Ordering::AcqRel) {
            0 => None,
            serial => Some(serial),
        }
    }

    /// Make the handle inert. Called when the plugin instance is destroyed,
    /// while the host pointer is still valid.
    ///
    /// Returns only once no call is inside the host any more; from then on
    /// every method is a no-op. The wait is bounded by the host's own
    /// `[thread-safe]` request callbacks, which only schedule work. It must
    /// not run on a thread that is itself inside one of this handle's calls.
    pub(crate) fn retire(&self) {
        self.calls.fetch_or(RETIRED, Ordering::AcqRel);
        while self.calls.load(Ordering::Acquire) & !RETIRED != 0 {
            std::thread::yield_now();
        }
    }
}
