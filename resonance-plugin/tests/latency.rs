//! Host-side tests for the CLAP latency extension of the bridge.
//!
//! The real host (resonance-audio's `clap_host`) makes exactly one latency
//! query, immediately after `activate()` — the CLAP spec only defines
//! `latency.get()` while the plugin is active. But at activation the bridge
//! moves the plugin object out of the main thread into the audio processor,
//! so the main-thread latency extension can no longer ask the plugin
//! directly. These tests drive the bridge through the actual CLAP C ABI
//! (via clack-host, in-process) and pin the fix: `activate` captures
//! `latency_samples()` right after `initialize()` and the extension serves
//! that cached value, so the host's activation-time query sees the real
//! latency instead of 0.
//!
//! The test plugin reports latency only once initialized, mimicking
//! resonance-mastering whose DSP chain (and therefore latency) exists only
//! after `initialize()` runs.

use clack_extensions::latency::PluginLatency;
use clack_host::prelude::*;
use clack_plugin::entry::SinglePluginEntry;

use resonance_plugin::{
    ClapBridge, EventIterator, OutputBuffer, Param, ResonancePlugin, TempoInfo,
};

// ---------------------------------------------------------------------------
// Test plugin: latency is only known after initialize()
// ---------------------------------------------------------------------------

/// The latency the plugin reports once its DSP is set up.
const INIT_LATENCY: u32 = 4242;

fn no_param(_: usize) -> &'static dyn Param {
    unreachable!("test plugin declares zero params")
}

/// An effect whose latency becomes non-zero only after `initialize()`,
/// like resonance-mastering (chain built in `initialize`) — a query on a
/// freshly-constructed plugin reads 0.
struct InitLatencyEffect {
    initialized: bool,
}

impl ResonancePlugin for InitLatencyEffect {
    const CLAP_ID: &'static str = "test.init-latency";
    const NAME: &'static str = "InitLatency";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static std::ffi::CStr] =
        &[resonance_plugin::features::AUDIO_EFFECT];
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
            INIT_LATENCY
        } else {
            0
        }
    }
}

// ---------------------------------------------------------------------------
// Minimal clack host
// ---------------------------------------------------------------------------

struct TestHostShared;

impl SharedHandler<'_> for TestHostShared {
    fn request_restart(&self) {}
    fn request_process(&self) {}
    fn request_callback(&self) {}
}

struct TestHost;

impl HostHandlers for TestHost {
    type Shared<'a> = TestHostShared;
    type MainThread<'a> = ();
    type AudioProcessor<'a> = ();
}

fn instantiate() -> PluginInstance<TestHost> {
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<InitLatencyEffect>>>(
        c"resonance-test-latency.clap",
    )
    .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();

    PluginInstance::<TestHost>::new(
        |_| TestHostShared,
        |_| (),
        &entry,
        c"test.init-latency",
        &host_info,
    )
    .expect("plugin instantiation")
}

/// The exact activation config the real host uses (`clap_host/bundle.rs`):
/// min 32 / max 8192 frames.
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The host's real sequence: activate, then query once. The plugin object
/// has moved to the audio processor by then; the extension must serve the
/// value captured post-`initialize()` — not the stale 0.
#[test]
fn activation_time_query_returns_real_latency() {
    let mut instance = instantiate();

    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");

    // This is the single query resonance-audio's host makes after activate.
    assert_eq!(query_latency(&mut instance), INIT_LATENCY);

    instance.deactivate(processor);
}

/// Before the first activation the plugin is uninitialized, so a direct
/// query reads 0 — this is why `activate` must capture the latency *after*
/// `initialize()` rather than serving any pre-activation snapshot.
#[test]
fn pre_activation_query_reads_uninitialized_plugin() {
    let mut instance = instantiate();

    assert_eq!(query_latency(&mut instance), 0);

    // Activation still fixes it up for the host's post-activation query.
    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");
    assert_eq!(query_latency(&mut instance), INIT_LATENCY);

    instance.deactivate(processor);
}

/// After deactivation the plugin object returns to the main thread and is
/// still initialized: both the direct path and a later re-activation keep
/// reporting the real latency.
#[test]
fn latency_survives_deactivate_and_reactivate() {
    let mut instance = instantiate();

    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");
    instance.deactivate(processor);

    // Inactive again: the extension asks the (initialized) plugin directly.
    assert_eq!(query_latency(&mut instance), INIT_LATENCY);

    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("re-activation");
    assert_eq!(query_latency(&mut instance), INIT_LATENCY);

    instance.deactivate(processor);
}
