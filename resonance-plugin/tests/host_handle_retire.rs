//! `HostHandle` liveness across instance destruction (code review PLG-07).
//!
//! The handle keeps the `clap_host` pointer with its lifetime erased and
//! goes inert when the instance is destroyed, which is what lets a plugin
//! leak a clone into an editor thread that outlives the instance. That
//! guarantee used to be a check-then-use: a thread that passed the
//! liveness check just before `retire()` could still be inside the host
//! callback while the host freed its data.
//!
//! The test host's `request_callback` sleeps, so a call is reliably in
//! flight while the main thread destroys the instance. The destroy must not
//! return while that call is still running, and no call may start after it.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use clack_host::prelude::*;
use clack_plugin::entry::SinglePluginEntry;

use resonance_plugin::{
    ClapBridge, EventIterator, HostHandle, OutputBuffer, Param, ResonancePlugin, TempoInfo,
};

/// How long the host's `request_callback` stays inside the call.
const CALL_TIME: Duration = Duration::from_millis(150);

static HANDLE: OnceLock<Arc<HostHandle>> = OnceLock::new();
/// A host callback is running right now.
static IN_CALL: AtomicBool = AtomicBool::new(false);
/// Host callbacks that started, in total.
static CALLS: AtomicU32 = AtomicU32::new(0);
/// Set by the test once the instance's destruction has returned.
static DESTROYED: AtomicBool = AtomicBool::new(false);
/// Host-callback work observed after the destruction returned — i.e. the
/// host's data would already have been freed under it.
static AFTER_DESTROY: AtomicU32 = AtomicU32::new(0);

fn no_param(_: usize) -> &'static dyn Param {
    unreachable!("test plugin declares zero params")
}

struct LeakyPlugin;

impl ResonancePlugin for LeakyPlugin {
    const CLAP_ID: &'static str = "test.host-handle-retire";
    const NAME: &'static str = "HostHandleRetire";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static std::ffi::CStr] =
        &[resonance_plugin::features::AUDIO_EFFECT];
    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self
    }
    fn param_count(&self) -> usize {
        0
    }
    fn param(&self, index: usize) -> &dyn Param {
        no_param(index)
    }
    fn initialize(&mut self, _sample_rate: f32, _max_buffer_size: u32) -> bool {
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
    fn set_host(&mut self, host: Arc<HostHandle>) {
        let _ = HANDLE.set(host);
    }
}

struct SlowHostShared;

impl SharedHandler<'_> for SlowHostShared {
    fn request_restart(&self) {}
    fn request_process(&self) {}
    fn request_callback(&self) {
        // Statics only: after a (buggy) destroy `self` would dangle.
        CALLS.fetch_add(1, Ordering::SeqCst);
        IN_CALL.store(true, Ordering::SeqCst);
        thread::sleep(CALL_TIME);
        if DESTROYED.load(Ordering::SeqCst) {
            AFTER_DESTROY.fetch_add(1, Ordering::SeqCst);
        }
        IN_CALL.store(false, Ordering::SeqCst);
    }
}

struct SlowHost;

impl HostHandlers for SlowHost {
    type Shared<'a> = SlowHostShared;
    type MainThread<'a> = ();
    type AudioProcessor<'a> = ();
}

#[test]
fn destroying_the_instance_waits_for_an_in_flight_host_call() {
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<LeakyPlugin>>>(
        c"host-handle-retire.clap",
    )
    .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();
    let instance = PluginInstance::<SlowHost>::new(
        |_| SlowHostShared,
        |_| (),
        &entry,
        c"test.host-handle-retire",
        &host_info,
    )
    .expect("plugin instantiation");

    // The "editor thread" that outlives the instance, holding a clone.
    let handle = Arc::clone(HANDLE.get().expect("the bridge hands the plugin a handle"));
    let editor = {
        let handle = Arc::clone(&handle);
        thread::spawn(move || handle.request_callback())
    };

    let deadline = Instant::now() + Duration::from_secs(10);
    while !IN_CALL.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "the host call never started");
        thread::yield_now();
    }

    // Destroy while the call is in flight.
    drop(instance);
    DESTROYED.store(true, Ordering::SeqCst);
    editor.join().expect("editor thread panicked");

    assert_eq!(
        AFTER_DESTROY.load(Ordering::SeqCst),
        0,
        "instance destruction returned while a host call was still running"
    );

    // And a call after destruction is a no-op.
    let calls = CALLS.load(Ordering::SeqCst);
    handle.request_callback();
    handle.request_restart();
    handle.set_latency_samples(1234);
    assert_eq!(CALLS.load(Ordering::SeqCst), calls, "a retired handle called the host");
}
