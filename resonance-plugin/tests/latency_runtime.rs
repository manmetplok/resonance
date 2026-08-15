//! Host-side tests for a **runtime** latency change (ba todo #1296).
//!
//! `tests/latency.rs` covers the static case: a plugin whose latency is
//! constant per activation. This file covers the one a lookahead limiter, an
//! IR block-size switch or an oversampling selector needs — the latency
//! changing *while the plugin is loaded* — which a host cannot discover on
//! its own: CLAP only defines `clap_plugin_latency.get()` while the plugin is
//! active, and by then the bridge has moved the plugin object into the audio
//! processor. The plugin has to push.
//!
//! Everything here is driven across the real CLAP C ABI (clack-host,
//! in-process), in both directions: the plugin's `HostHandle` calls
//! `clap_host.request_restart` / `request_callback` through the host vtable,
//! the bridge calls `clap_host_latency.changed()` from `on_main_thread`, and
//! the host reads the new figure back through `clap_plugin_latency.get()`.
//! That last read is exactly what the real host (`resonance-audio`'s
//! `clap_host`) does when it services a restart request, and what it feeds
//! into plugin delay compensation.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

use clack_extensions::latency::{HostLatency, HostLatencyImpl, PluginLatency};
use clack_host::prelude::*;
use clack_plugin::entry::SinglePluginEntry;

use resonance_plugin::{
    ClapBridge, EventIterator, HostHandle, OutputBuffer, Param, ResonancePlugin, TempoInfo,
};

// ---------------------------------------------------------------------------
// Test plugin: latency changes at runtime, like a limiter's lookahead knob
// ---------------------------------------------------------------------------

/// Latency after `initialize()`, before anything changes it.
const INITIAL_LATENCY: u32 = 64;
/// What the "lookahead knob" moves it to.
const NEW_LATENCY: u32 = 512;

/// The latency the plugin currently reports. Stands in for DSP state a real
/// plugin would own (`self.limiter.lookahead_samples()`).
static REPORTED: AtomicU32 = AtomicU32::new(INITIAL_LATENCY);

/// The host handle the bridge gave the plugin, parked where the test can
/// reach it. A real plugin keeps this in a field and hands a clone to its
/// editor; the knob callback then does exactly what `move_lookahead` does.
static HANDLE: OnceLock<Arc<HostHandle>> = OnceLock::new();

/// The plugin-side action under test: change the DSP latency and tell the
/// host. This is the whole of what a lookahead knob's callback has to do.
fn move_lookahead(samples: u32) {
    REPORTED.store(samples, Ordering::Release);
    HANDLE
        .get()
        .expect("the bridge must hand the plugin a host handle at construction")
        .set_latency_samples(samples);
}

fn no_param(_: usize) -> &'static dyn Param {
    unreachable!("test plugin declares zero params")
}

struct RuntimeLatencyEffect {
    initialized: bool,
}

impl ResonancePlugin for RuntimeLatencyEffect {
    const CLAP_ID: &'static str = "test.runtime-latency";
    const NAME: &'static str = "RuntimeLatency";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static str] = &[];
    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self { initialized: false }
    }
    fn param_count(&self) -> usize {
        0
    }
    fn param(&self, index: usize) -> &dyn Param {
        no_param(index)
    }
    fn initialize(&mut self, _sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.initialized = true;
        true
    }
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _outputs: &mut [OutputBuffer<'_>],
        _frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
    }
    fn latency_samples(&self) -> u32 {
        if self.initialized {
            REPORTED.load(Ordering::Acquire)
        } else {
            0
        }
    }
    fn set_host(&mut self, host: Arc<HostHandle>) {
        // `new_main_thread` builds one plugin per instance; the test only
        // ever makes one, so the first handle is the live one.
        let _ = HANDLE.set(host);
    }
}

// ---------------------------------------------------------------------------
// A clack host that actually implements the latency extension
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestHostShared {
    restart_requested: AtomicBool,
    callback_requested: AtomicBool,
    /// Set from `HostLatencyImpl::changed`, i.e. from the main-thread
    /// handler; kept here so the test can read it off the instance.
    latency_changed: AtomicBool,
}

impl SharedHandler<'_> for TestHostShared {
    fn request_restart(&self) {
        self.restart_requested.store(true, Ordering::SeqCst)
    }
    fn request_process(&self) {}
    fn request_callback(&self) {
        self.callback_requested.store(true, Ordering::SeqCst)
    }
}

struct TestHostMainThread<'a> {
    shared: &'a TestHostShared,
}

impl<'a> MainThreadHandler<'a> for TestHostMainThread<'a> {}

impl HostLatencyImpl for TestHostMainThread<'_> {
    fn changed(&mut self) {
        self.shared.latency_changed.store(true, Ordering::SeqCst)
    }
}

struct TestHost;

impl HostHandlers for TestHost {
    type Shared<'a> = TestHostShared;
    type MainThread<'a> = TestHostMainThread<'a>;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        builder.register::<HostLatency>();
    }
}

fn instantiate() -> PluginInstance<TestHost> {
    let entry =
        PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<RuntimeLatencyEffect>>>(
            c"resonance-test-latency-runtime.clap",
        )
        .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();

    PluginInstance::<TestHost>::new(
        |_| TestHostShared::default(),
        |shared| TestHostMainThread { shared },
        &entry,
        c"test.runtime-latency",
        &host_info,
    )
    .expect("plugin instantiation")
}

/// The exact activation config the real host uses (`clap_host/bundle.rs`).
fn audio_config() -> PluginAudioConfiguration {
    PluginAudioConfiguration {
        sample_rate: 48_000.0,
        min_frames_count: 32,
        max_frames_count: 8192,
    }
}

fn query_latency(instance: &mut PluginInstance<TestHost>) -> u32 {
    let ext = instance
        .plugin_shared_handle()
        .get_extension::<PluginLatency>()
        .expect("bridge must expose the latency extension");
    ext.get(&mut instance.plugin_handle())
}

/// Take one of the host-side flags, clearing it.
fn take_flag(instance: &PluginInstance<TestHost>, pick: fn(&TestHostShared) -> &AtomicBool) -> bool {
    instance
        .access_shared_handler(pick)
        .swap(false, Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

/// The whole runtime-latency loop, in the order a host and plugin perform it.
///
/// One test, because the plugin's reported latency and its host handle are
/// process-wide statics (the plugin object itself is unreachable from here —
/// it lives inside the bridge), so splitting this would let the cases race.
#[test]
fn a_runtime_latency_change_reaches_the_host_and_survives_the_restart() {
    let mut instance = instantiate();

    // -- the plugin got a handle at construction, before activation --------
    assert!(
        HANDLE.get().is_some(),
        "the bridge must hand every plugin a host handle in `set_host`"
    );

    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");
    assert_eq!(
        query_latency(&mut instance),
        INITIAL_LATENCY,
        "the activation-time query serves what the plugin reported after initialize()"
    );
    // Nothing has asked the host for anything yet.
    assert!(!take_flag(&instance, |h| &h.restart_requested));
    assert!(!take_flag(&instance, |h| &h.callback_requested));
    assert!(!take_flag(&instance, |h| &h.latency_changed));

    // -- the knob moves, while the plugin is active ------------------------
    move_lookahead(NEW_LATENCY);

    // CLAP only lets the reported latency change while deactivated, so the
    // bridge must ask for the deactivate -> reactivate cycle. This is the
    // signal the real host services (`take_host_restart_request`), and the
    // one that makes it re-read the latency and republish PDC.
    assert!(
        take_flag(&instance, |h| &h.restart_requested),
        "a latency change while active must request a restart"
    );
    // …and ask to be called back on the main thread, because
    // `clap_host_latency.changed()` is [main-thread] and the change can come
    // from the audio thread.
    assert!(
        take_flag(&instance, |h| &h.callback_requested),
        "a latency change must request a main-thread callback"
    );
    assert!(
        !take_flag(&instance, |h| &h.latency_changed),
        "the plugin must not call the [main-thread] `changed()` from wherever it happened to be"
    );

    // A query right now — before the host has done anything about it — must
    // already report the new figure. The plugin object is in the audio
    // processor and unreachable from the main thread, so this can only come
    // from what the plugin pushed.
    assert_eq!(
        query_latency(&mut instance),
        NEW_LATENCY,
        "an active latency query must serve the pushed value, not the activation-time one"
    );

    // -- the host runs the requested main-thread callback ------------------
    instance.call_on_main_thread_callback();
    assert!(
        take_flag(&instance, |h| &h.latency_changed),
        "`on_main_thread` must call clap_host_latency.changed() so the host re-queries"
    );

    // Draining is one-shot: a second callback with nothing pending must not
    // spam the host.
    instance.call_on_main_thread_callback();
    assert!(!take_flag(&instance, |h| &h.latency_changed));

    // -- the host services the restart, exactly as resonance-audio does ----
    instance.deactivate(processor);
    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("re-activation");
    assert_eq!(
        query_latency(&mut instance),
        NEW_LATENCY,
        "the re-activation must re-read the plugin's new latency"
    );

    // -- a redundant report is silent --------------------------------------
    move_lookahead(NEW_LATENCY);
    assert!(
        !take_flag(&instance, |h| &h.restart_requested),
        "reporting the same latency again must not cycle the plugin"
    );
    assert!(!take_flag(&instance, |h| &h.callback_requested));

    // -- a change while inactive needs no restart, only a notification -----
    instance.deactivate(processor);
    move_lookahead(INITIAL_LATENCY);
    assert!(
        !take_flag(&instance, |h| &h.restart_requested),
        "an inactive plugin's latency may change in place — no restart needed"
    );
    assert!(take_flag(&instance, |h| &h.callback_requested));
    instance.call_on_main_thread_callback();
    assert!(take_flag(&instance, |h| &h.latency_changed));

    // The inactive query goes straight to the plugin object and agrees.
    assert_eq!(query_latency(&mut instance), INITIAL_LATENCY);
}
