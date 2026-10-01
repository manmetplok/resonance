//! `HostHandle::request_params_rescan` never lets the host re-read ahead
//! of the value it announces (drums-plugin-rework.md §5.4, review finding
//! 4).
//!
//! While active, an ordinary param's value reaches what `get_value` serves
//! only through the bridge's per-block push-back, which runs BEFORE the
//! plugin's `process()`. A rescan requested inside `process()` used to be
//! posted at once, and a host that services callbacks between blocks (as
//! Resonance's engine does) re-read the mirror before the next block's
//! push-back — the old value, after being told it changed. And an
//! inactive plugin's rescan was never published into the mirror at all.
//!
//! The host here implements `clap_host_params` for real and services
//! `request_callback` between blocks, the way the engine does.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use clack_extensions::params::{
    HostParams, HostParamsImplMainThread, HostParamsImplShared, ParamClearFlags,
    ParamRescanFlags, PluginParams,
};
use clack_host::prelude::*;
use clack_plugin::entry::SinglePluginEntry;

use resonance_plugin::{
    stable_hash, ClapBridge, EventIterator, FloatParam, FloatRange, HostHandle, OutputBuffer,
    Param, ResonancePlugin, TempoInfo,
};

const FRAMES: usize = 64;

/// The value `process()` moves `gain` to, when asked.
const MOVED: f32 = 0.75;

thread_local! {
    /// The plugin's host handle and its shared `gain`, as `set_host` saw
    /// them. Per thread: each test builds (and processes) its instance on
    /// its own thread.
    static HANDLE: RefCell<Option<(Arc<HostHandle>, Arc<FloatParam>)>> =
        const { RefCell::new(None) };
    /// Set by the test: the next `process()` moves `gain` itself and asks
    /// for a rescan, from inside the block.
    static MOVE_IN_PROCESS: Cell<bool> = const { Cell::new(false) };
}

fn handle() -> (Arc<HostHandle>, Arc<FloatParam>) {
    HANDLE.with(|h| h.borrow().clone()).expect("set_host ran")
}

struct SelfMovingPlugin {
    gain: Arc<FloatParam>,
}

impl ResonancePlugin for SelfMovingPlugin {
    const CLAP_ID: &'static str = "test.params-rescan";
    const NAME: &'static str = "ParamsRescan";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static std::ffi::CStr] =
        &[resonance_plugin::features::AUDIO_EFFECT];
    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            gain: Arc::new(FloatParam::new(
                "gain",
                "Gain",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )),
        }
    }
    fn param_count(&self) -> usize {
        1
    }
    fn param(&self, _index: usize) -> &dyn Param {
        &*self.gain
    }
    fn initialize(&mut self, _sample_rate: f32, _max_buffer_size: u32) -> bool {
        true
    }
    fn reset(&mut self) {}
    fn set_host(&mut self, host: Arc<HostHandle>) {
        HANDLE.with(|h| *h.borrow_mut() = Some((host, self.gain.clone())));
    }
    fn process(
        &mut self,
        _outputs: &mut [OutputBuffer<'_>],
        _frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        if MOVE_IN_PROCESS.with(|m| m.replace(false)) {
            self.gain.set_value(MOVED);
            handle().0.request_params_rescan();
        }
    }
}

#[derive(Default)]
struct Shared {
    callback_requested: AtomicBool,
}

impl SharedHandler<'_> for Shared {
    fn request_restart(&self) {}
    fn request_process(&self) {}
    fn request_callback(&self) {
        self.callback_requested.store(true, Ordering::SeqCst);
    }
}

impl HostParamsImplShared for Shared {
    fn request_flush(&self) {}
}

struct MainThread<'a> {
    #[allow(dead_code)]
    shared: &'a Shared,
    /// `rescan` flags received, OR-ed.
    rescans: Arc<AtomicU32>,
}

impl<'a> MainThreadHandler<'a> for MainThread<'a> {}

impl HostParamsImplMainThread for MainThread<'_> {
    fn rescan(&mut self, flags: ParamRescanFlags) {
        self.rescans.fetch_or(flags.bits(), Ordering::SeqCst);
    }
    fn clear(&mut self, _param_id: ClapId, _flags: ParamClearFlags) {}
}

struct Host;

impl HostHandlers for Host {
    type Shared<'a> = Shared;
    type MainThread<'a> = MainThread<'a>;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        builder.register::<HostParams>();
    }
}

fn instance(rescans: Arc<AtomicU32>) -> PluginInstance<Host> {
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<SelfMovingPlugin>>>(
        c"resonance-test-params-rescan.clap",
    )
    .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();
    PluginInstance::<Host>::new(
        |_| Shared::default(),
        move |shared| MainThread { shared, rescans },
        &entry,
        c"test.params-rescan",
        &host_info,
    )
    .expect("plugin instantiation")
}

fn gain(instance: &mut PluginInstance<Host>) -> f64 {
    let ext = instance
        .plugin_shared_handle()
        .get_extension::<PluginParams>()
        .expect("params");
    ext.get_value(&mut instance.plugin_handle(), ClapId::new(stable_hash("gain")))
        .expect("gain")
}

/// The engine's poll: run a requested callback, and if it carried a
/// rescan, re-read the value — what the host's mirror would take.
fn service(instance: &mut PluginInstance<Host>, rescans: &AtomicU32) -> Option<f64> {
    let requested = instance
        .access_shared_handler(|s| s.callback_requested.swap(false, Ordering::SeqCst));
    if requested {
        instance.call_on_main_thread_callback();
    }
    (rescans.swap(0, Ordering::SeqCst) != 0).then(|| gain(instance))
}

#[test]
fn a_rescan_requested_inside_process_reaches_the_host_after_the_value() {
    let rescans = Arc::new(AtomicU32::new(0));
    let mut instance = instance(rescans.clone());
    let mut processor = instance
        .activate(
            |_, _| (),
            PluginAudioConfiguration {
                sample_rate: 48_000.0,
                min_frames_count: 1,
                max_frames_count: FRAMES as u32,
            },
        )
        .expect("activate")
        .start_processing()
        .expect("start");

    let mut ports = AudioPorts::with_capacity(2, 1);
    let mut buffers = [[0.0f32; FRAMES]; 2];
    MOVE_IN_PROCESS.with(|m| m.set(true));
    {
        let mut outputs = ports.with_output_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_output_only(
                buffers.iter_mut().map(|b| b.as_mut_slice()),
            ),
        }]);
        processor
            .process(
                &InputAudioBuffers::empty(),
                &mut outputs,
                &EventBuffer::new().as_input(),
                &mut EventBuffer::new().as_output(),
                None,
                None,
            )
            .expect("process");
    }

    // Between blocks, exactly where the engine polls.
    let reread = service(&mut instance, &rescans).expect("the rescan reached the host");
    assert_eq!(
        reread, MOVED as f64,
        "the host re-read the value the rescan announced, not the one before it"
    );
    instance.deactivate(processor.stop_processing());
}

#[test]
fn an_inactive_plugins_rescan_publishes_its_values_first() {
    let rescans = Arc::new(AtomicU32::new(0));
    let mut instance = instance(rescans.clone());
    let (handle, shared_gain) = handle();
    // Moved off the audio thread with no block running (the plugin is not
    // even active): a selection derived from a load, say.
    shared_gain.set_value(0.25);
    handle.request_params_rescan();

    let reread = service(&mut instance, &rescans).expect("the rescan reached the host");
    assert_eq!(reread, 0.25);
    let flags = ParamRescanFlags::from_bits_truncate(rescans.load(Ordering::SeqCst));
    assert!(flags.is_empty(), "consumed");
}
