//! A host may call back into the plugin from inside a host function the
//! plugin called, and the bridge must serve that call.
//!
//! CLAP allows it and real hosts do it: they answer
//! `clap_host_params.rescan()` by re-reading the values, they answer
//! `clap_host_latency.changed()` (made during `activate`) by asking for the
//! latency, and they answer `clap_host_gui.closed()` by destroying the
//! editor. Each of those is a main-thread plugin call that arrives while
//! another main-thread plugin call is still on the stack.
//!
//! Under clack 0.1 every main-thread call was handed a `&mut` to the
//! bridge's `ClapMainThread`, so each re-entry aliased a live `&mut` —
//! undefined behaviour that happened to work. clack 0.2 hands out `&`, and
//! the bridge keeps its mutable state in cells under one rule: no borrow
//! is held across a call that can re-enter. This file is the guard on that
//! rule. A borrow held across one of these host calls makes the re-entrant
//! call fail its `try_borrow` (or panic inside clack's FFI guard), which
//! shows up here as a wrong value, a refused destroy or a missing drop.
//!
//! The host below makes the re-entrant calls through the raw CLAP vtables,
//! the way `resonance-audio`'s host does: clack-host only hands out a
//! main-thread plugin handle to the code that owns the instance, not to a
//! host callback.

use std::cell::{Cell, RefCell};
use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use clack_extensions::gui::{
    GuiApiType, GuiConfiguration, GuiSize, HostGui, HostGuiImpl, PluginGui,
};
use clack_extensions::latency::{HostLatency, HostLatencyImpl};
use clack_extensions::params::{
    HostParams, HostParamsImplMainThread, HostParamsImplShared, ParamClearFlags, ParamRescanFlags,
};
use clack_host::prelude::*;
use clack_plugin::entry::SinglePluginEntry;
use clap_sys::ext::gui::{clap_plugin_gui, CLAP_EXT_GUI};
use clap_sys::ext::latency::{clap_plugin_latency, CLAP_EXT_LATENCY};
use clap_sys::ext::params::{clap_plugin_params, CLAP_EXT_PARAMS};
use clap_sys::plugin::clap_plugin;

use resonance_plugin::gui::{EditorFactory, PluginEditor};
use resonance_plugin::{
    stable_hash, ClapBridge, EventIterator, FloatParam, FloatRange, HostHandle, OutputBuffer,
    Param, ResonancePlugin, TempoInfo,
};

// ---------------------------------------------------------------------------
// The plugin
// ---------------------------------------------------------------------------

/// The latency the plugin reports once it has been told to change it.
const LATENCY: u32 = 256;

thread_local! {
    /// The plugin's host handle and its `gain`, as `set_host` saw them.
    /// Per thread: each test builds its instance on its own thread.
    static HANDLE: RefCell<Option<(Arc<HostHandle>, Arc<FloatParam>)>> =
        const { RefCell::new(None) };
    /// What `latency_samples` reports.
    static REPORTED_LATENCY: Cell<u32> = const { Cell::new(0) };
}

fn handle() -> (Arc<HostHandle>, Arc<FloatParam>) {
    HANDLE.with(|h| h.borrow().clone()).expect("set_host ran")
}

type Closer = Box<dyn FnOnce() + Send>;

/// The close callback the bridge handed the open editor, and whether the
/// editor has been dropped. Process-wide (the editor is `Send`, so the
/// bridge may in principle move it); only the gui test touches them.
static CLOSER: Mutex<Option<Closer>> = Mutex::new(None);
static EDITOR_DROPPED: AtomicBool = AtomicBool::new(false);

struct FakeEditor;

impl PluginEditor for FakeEditor {
    fn show(&mut self) {}
    fn hide(&mut self) {}
    fn size(&self) -> (u32, u32) {
        (400, 300)
    }
    fn set_size(&mut self, _: u32, _: u32) -> bool {
        true
    }
    fn can_resize(&self) -> bool {
        false
    }
    fn set_closed_callback(&mut self, on_closed: Closer) {
        *CLOSER.lock().unwrap() = Some(on_closed);
    }
}

impl Drop for FakeEditor {
    fn drop(&mut self) {
        EDITOR_DROPPED.store(true, Ordering::SeqCst);
    }
}

struct FakeFactory;

impl EditorFactory for FakeFactory {
    fn supports(&self, api: &str, floating: bool) -> bool {
        api == "wayland" && floating
    }
    fn preferred(&self) -> Option<(&'static str, bool)> {
        Some(("wayland", true))
    }
    fn preferred_size(&self) -> (u32, u32) {
        (400, 300)
    }
    fn create(&self, api: &str, floating: bool) -> Option<Box<dyn PluginEditor>> {
        self.supports(api, floating)
            .then(|| Box::new(FakeEditor) as Box<dyn PluginEditor>)
    }
}

struct ReentrantProbe {
    gain: Arc<FloatParam>,
}

impl ResonancePlugin for ReentrantProbe {
    const CLAP_ID: &'static str = "test.reentrancy";
    const NAME: &'static str = "Reentrancy";
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
    fn process(
        &mut self,
        _outputs: &mut [OutputBuffer<'_>],
        _frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
    }
    fn latency_samples(&self) -> u32 {
        REPORTED_LATENCY.with(Cell::get)
    }
    fn set_host(&mut self, host: Arc<HostHandle>) {
        HANDLE.with(|h| *h.borrow_mut() = Some((host, self.gain.clone())));
    }
    fn editor_factory(&self) -> Option<Arc<dyn EditorFactory>> {
        Some(Arc::new(FakeFactory))
    }
}

// ---------------------------------------------------------------------------
// Raw re-entrant calls
// ---------------------------------------------------------------------------

/// The plugin's extension vtable `id`, through the raw `clap_plugin`.
///
/// # Safety
///
/// `plugin` must be a live plugin instance, and `T` the C struct CLAP
/// defines for `id`.
unsafe fn plugin_ext<T>(plugin: *const clap_plugin, id: &CStr) -> &'static T {
    // SAFETY: the caller vouches for `plugin`; `get_extension` is callable
    // from any thread once the plugin is initialised, which it is by the
    // time any host callback below runs.
    unsafe {
        let get = (*plugin).get_extension.expect("clap_plugin.get_extension");
        let ext = get(plugin, id.as_ptr()).cast::<T>();
        assert!(!ext.is_null(), "the bridge exposes {id:?}");
        // The vtable is a static in the plugin binary, which lives as long
        // as the process here (it is this test binary).
        &*ext
    }
}

// ---------------------------------------------------------------------------
// The host
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Shared {
    /// The plugin instance, captured while it initialises.
    plugin: AtomicPtr<clap_plugin>,
    callback_requested: AtomicBool,
    closed_calls: AtomicU32,
    /// `get_value` answered from inside `rescan`, as `f64` bits.
    rescan_read: AtomicU64,
    rescan_read_ok: AtomicBool,
    /// `latency.get()` answered from inside `latency.changed()`.
    latency_read: AtomicU32,
    latency_changed: AtomicBool,
}

impl Shared {
    fn plugin(&self) -> *const clap_plugin {
        let plugin = self.plugin.load(Ordering::SeqCst);
        assert!(!plugin.is_null(), "initializing() captured the plugin");
        plugin
    }
}

impl<'a> SharedHandler<'a> for Shared {
    fn initializing(&self, instance: InitializingPluginHandle<'a>) {
        self.plugin
            .store(instance.as_raw_ptr().cast_mut(), Ordering::SeqCst);
    }
    fn request_restart(&self) {}
    fn request_process(&self) {}
    fn request_callback(&self) {
        self.callback_requested.store(true, Ordering::SeqCst);
    }
}

impl HostParamsImplShared for Shared {
    fn request_flush(&self) {}
}

impl HostGuiImpl for Shared {
    fn resize_hints_changed(&self) {}
    fn request_resize(&self, _: GuiSize) -> Result<(), HostError> {
        Err(HostError::Message("floating only"))
    }
    fn request_show(&self) -> Result<(), HostError> {
        Err(HostError::Message("refused"))
    }
    fn request_hide(&self) -> Result<(), HostError> {
        Err(HostError::Message("refused"))
    }
    /// What a host does on `closed`: destroy the editor, right away, from
    /// inside the plugin's `on_main_thread` that reported it.
    fn closed(&self, _was_destroyed: bool) {
        self.closed_calls.fetch_add(1, Ordering::SeqCst);
        let plugin = self.plugin();
        // SAFETY: `closed` is [main-thread] and runs inside the plugin's
        // `on_main_thread`, so the instance is alive and this is the main
        // thread, which is what `gui.destroy` requires.
        unsafe {
            let gui = plugin_ext::<clap_plugin_gui>(plugin, CLAP_EXT_GUI);
            gui.destroy.expect("gui.destroy")(plugin);
        }
    }
}

struct MainThread<'a> {
    shared: &'a Shared,
}

impl<'a> MainThreadHandler<'a> for MainThread<'a> {}

impl HostParamsImplMainThread for MainThread<'_> {
    /// What a host does on `rescan`: read the values back, right away.
    fn rescan(&self, _flags: ParamRescanFlags) {
        let plugin = self.shared.plugin();
        let mut value = f64::NAN;
        // SAFETY: `rescan` is [main-thread] and runs inside the plugin's
        // `on_main_thread`: the instance is alive, and `get_value` is a
        // main-thread call. `value` outlives the call.
        let ok = unsafe {
            let params = plugin_ext::<clap_plugin_params>(plugin, CLAP_EXT_PARAMS);
            params.get_value.expect("params.get_value")(plugin, stable_hash("gain"), &mut value)
        };
        self.shared.rescan_read_ok.store(ok, Ordering::SeqCst);
        self.shared
            .rescan_read
            .store(value.to_bits(), Ordering::SeqCst);
    }
    fn clear(&self, _param_id: ClapId, _flags: ParamClearFlags) {}
}

impl HostLatencyImpl for MainThread<'_> {
    /// What a host does on `changed` during activation: re-read it.
    fn changed(&self) {
        let plugin = self.shared.plugin();
        // SAFETY: `changed` is [main-thread] and runs inside the plugin's
        // `activate`: the instance is alive and this is the main thread.
        let latency = unsafe {
            let latency = plugin_ext::<clap_plugin_latency>(plugin, CLAP_EXT_LATENCY);
            latency.get.expect("latency.get")(plugin)
        };
        self.shared.latency_read.store(latency, Ordering::SeqCst);
        self.shared.latency_changed.store(true, Ordering::SeqCst);
    }
}

struct Host;

impl HostHandlers for Host {
    type Shared<'a> = Shared;
    type MainThread<'a> = MainThread<'a>;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        builder
            .register::<HostParams>()
            .register::<HostLatency>()
            .register::<HostGui>();
    }
}

fn instantiate() -> PluginInstance<Host> {
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<ReentrantProbe>>>(
        c"resonance-test-reentrancy.clap",
    )
    .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();
    PluginInstance::<Host>::new(
        |_| Shared::default(),
        |shared| MainThread { shared },
        &entry,
        c"test.reentrancy",
        &host_info,
    )
    .expect("plugin instantiation")
}

fn take_callback_request(instance: &PluginInstance<Host>) -> bool {
    instance
        .access_shared_handler(|h| &h.callback_requested)
        .swap(false, Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// `on_main_thread` publishes the inactive plugin's values and calls
/// `rescan`; the host's `get_value` from inside it is served the new value.
#[test]
fn get_value_from_inside_rescan_reads_the_rescanned_value() {
    let mut instance = instantiate();
    let (host, gain) = handle();
    gain.set_value(0.8);
    host.request_params_rescan();
    assert!(take_callback_request(&instance));

    instance.call_on_main_thread_callback();

    let read = instance.access_shared_handler(|h| {
        (
            h.rescan_read_ok.load(Ordering::SeqCst),
            f64::from_bits(h.rescan_read.load(Ordering::SeqCst)),
        )
    });
    assert!(read.0, "a re-entrant get_value must be answered");
    assert!(
        (read.1 - 0.8).abs() < 1e-6,
        "a re-entrant get_value must read the value the rescan announced, got {}",
        read.1
    );
}

/// A latency change reported while inactive is announced during
/// `activate`; the host's `latency.get()` from inside that `changed()` is
/// answered with the new figure, and the activation still succeeds.
#[test]
fn latency_get_from_inside_activate_reads_the_new_latency() {
    let mut instance = instantiate();
    REPORTED_LATENCY.with(|l| l.set(LATENCY));
    handle().0.set_latency_samples(LATENCY);

    let processor = instance
        .activate(
            |_, _| (),
            PluginAudioConfiguration {
                sample_rate: 48_000.0,
                min_frames_count: 32,
                max_frames_count: 1024,
            },
        )
        .expect("activation must survive a re-entrant latency query");

    let (changed, read) = instance.access_shared_handler(|h| {
        (
            h.latency_changed.load(Ordering::SeqCst),
            h.latency_read.load(Ordering::SeqCst),
        )
    });
    assert!(changed, "the inactive change must be announced during activate");
    assert_eq!(read, LATENCY, "the re-entrant latency.get() must see the new latency");

    instance.deactivate(processor);
}

/// A self-closed editor is reported from `on_main_thread`; the host
/// destroys it from inside `closed()`, and the editor really goes.
#[test]
fn destroy_from_inside_closed_drops_the_editor() {
    let mut instance = instantiate();
    let gui = instance
        .plugin_shared_handle()
        .get_extension::<PluginGui>()
        .expect("gui extension");
    let config = GuiConfiguration {
        api_type: GuiApiType::WAYLAND,
        is_floating: true,
    };
    gui.create(&instance.plugin_handle(), config)
        .expect("gui create");
    gui.show(&instance.plugin_handle()).expect("gui show");

    // The runtime reports the close from its own thread.
    let closer = CLOSER.lock().unwrap().take().expect("a close callback");
    std::thread::spawn(closer).join().unwrap();
    assert!(take_callback_request(&instance));

    instance.call_on_main_thread_callback();

    assert_eq!(
        instance.access_shared_handler(|h| h.closed_calls.load(Ordering::SeqCst)),
        1
    );
    assert!(
        EDITOR_DROPPED.load(Ordering::SeqCst),
        "a destroy made from inside closed() must drop the editor"
    );

    // And the editor slot is usable again afterwards.
    gui.create(&instance.plugin_handle(), config)
        .expect("a create after the re-entrant destroy");
    gui.destroy(&instance.plugin_handle());
}
