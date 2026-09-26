//! A self-closed editor reaches the host as `clap_host_gui.closed()` (PLG-01).
//!
//! When the user closes a plugin editor from its own titlebar, the GUI
//! runtime tears the window down without the host having asked. CLAP
//! expects the plugin to say so through `clap_host_gui.closed()`, or the
//! host keeps the editor marked open. The bridge wires that up for every
//! plugin: it hands each editor a callback through
//! `PluginEditor::set_closed_callback`, latches the report on the
//! `HostHandle`, asks for a main-thread callback, and calls `closed(true)`
//! from `on_main_thread` (the call is `[main-thread]`; the runtime closes
//! on its own thread).
//!
//! Driven across the real CLAP C ABI (clack-host, in-process), with a fake
//! `PluginEditor` standing in for `RuntimeEditorHandle`, whose forwarding
//! to the runtime is the only part not exercised here.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use clack_extensions::gui::{
    GuiApiType, GuiConfiguration, GuiSize, HostGui, HostGuiImpl, PluginGui,
};
use clack_host::prelude::*;
use clack_plugin::entry::SinglePluginEntry;

use resonance_plugin::gui::{EditorFactory, PluginEditor};
use resonance_plugin::{
    ClapBridge, EventIterator, OutputBuffer, Param, ResonancePlugin, TempoInfo,
};

// ---------------------------------------------------------------------------
// A plugin whose editor can be "closed by the user" from the test
// ---------------------------------------------------------------------------

type Closer = Box<dyn FnOnce() + Send>;

/// The close callback of every editor the factory made, in creation order.
/// Calling one is what the runtime does when its window closes itself.
static CLOSERS: Mutex<Vec<Option<Closer>>> = Mutex::new(Vec::new());

/// Close the `n`-th editor (0-based) "from its own titlebar".
fn user_closes(n: usize) {
    let closer = CLOSERS.lock().unwrap()[n]
        .take()
        .expect("the bridge must hand every editor a close callback");
    closer();
}

struct FakeEditor {
    slot: usize,
}

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
    fn set_closed_callback(&mut self, on_closed: Box<dyn FnOnce() + Send>) {
        CLOSERS.lock().unwrap()[self.slot] = Some(on_closed);
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
        if !self.supports(api, floating) {
            return None;
        }
        let mut closers = CLOSERS.lock().unwrap();
        closers.push(None);
        Some(Box::new(FakeEditor {
            slot: closers.len() - 1,
        }))
    }
}

fn no_param(_: usize) -> &'static dyn Param {
    unreachable!("test plugin declares zero params")
}

struct EditorEffect;

impl ResonancePlugin for EditorEffect {
    const CLAP_ID: &'static str = "test.gui-closed";
    const NAME: &'static str = "GuiClosed";
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
    fn editor_factory(&self) -> Option<Arc<dyn EditorFactory>> {
        Some(Arc::new(FakeFactory))
    }
}

// ---------------------------------------------------------------------------
// A clack host that implements the gui extension
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestHostShared {
    callback_requested: AtomicBool,
    closed_calls: AtomicU32,
    last_was_destroyed: AtomicBool,
}

impl SharedHandler<'_> for TestHostShared {
    fn request_restart(&self) {}
    fn request_process(&self) {}
    fn request_callback(&self) {
        self.callback_requested.store(true, Ordering::SeqCst)
    }
}

impl HostGuiImpl for TestHostShared {
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
    fn closed(&self, was_destroyed: bool) {
        self.last_was_destroyed
            .store(was_destroyed, Ordering::SeqCst);
        self.closed_calls.fetch_add(1, Ordering::SeqCst);
    }
}

struct TestHostMainThread;

impl<'a> MainThreadHandler<'a> for TestHostMainThread {}

struct TestHost;

impl HostHandlers for TestHost {
    type Shared<'a> = TestHostShared;
    type MainThread<'a> = TestHostMainThread;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        builder.register::<HostGui>();
    }
}

fn instantiate() -> PluginInstance<TestHost> {
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<EditorEffect>>>(
        c"resonance-test-gui-closed.clap",
    )
    .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();

    PluginInstance::<TestHost>::new(
        |_| TestHostShared::default(),
        |_| TestHostMainThread,
        &entry,
        c"test.gui-closed",
        &host_info,
    )
    .expect("plugin instantiation")
}

fn plugin_gui(instance: &PluginInstance<TestHost>) -> PluginGui {
    instance
        .plugin_shared_handle()
        .get_extension::<PluginGui>()
        .expect("bridge must expose the gui extension")
}

fn open(instance: &mut PluginInstance<TestHost>) {
    let gui = plugin_gui(instance);
    let config = GuiConfiguration {
        api_type: GuiApiType::WAYLAND,
        is_floating: true,
    };
    gui.create(&mut instance.plugin_handle(), config)
        .expect("gui create");
    gui.show(&mut instance.plugin_handle()).expect("gui show");
}

fn destroy(instance: &mut PluginInstance<TestHost>) {
    let gui = plugin_gui(instance);
    gui.destroy(&mut instance.plugin_handle());
}

fn take_callback_request(instance: &PluginInstance<TestHost>) -> bool {
    instance
        .access_shared_handler(|h| &h.callback_requested)
        .swap(false, Ordering::SeqCst)
}

fn closed_calls(instance: &PluginInstance<TestHost>) -> u32 {
    instance
        .access_shared_handler(|h| &h.closed_calls)
        .load(Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

/// One test: the fake editors share a process-wide registry of callbacks,
/// so splitting the cases would let them race.
#[test]
fn a_user_closed_editor_notifies_the_host_once_and_only_for_the_live_editor() {
    let mut instance = instantiate();

    // -- editor 0: the user closes it from its own window ------------------
    open(&mut instance);
    assert!(!take_callback_request(&instance));

    // The runtime calls this from its own (editor) thread.
    std::thread::spawn(|| user_closes(0)).join().unwrap();

    assert_eq!(
        closed_calls(&instance),
        0,
        "`clap_host_gui.closed()` is [main-thread]; the runtime's thread must not call it"
    );
    assert!(
        take_callback_request(&instance),
        "a self-closed editor must ask the host for a main-thread callback"
    );
    instance.call_on_main_thread_callback();
    assert_eq!(
        closed_calls(&instance),
        1,
        "`on_main_thread` must deliver `clap_host_gui.closed()`"
    );
    assert!(
        instance
            .access_shared_handler(|h| &h.last_was_destroyed)
            .load(Ordering::SeqCst),
        "the runtime already tore the window down: was_destroyed must be true"
    );

    // One-shot: another callback with nothing pending must not repeat it.
    instance.call_on_main_thread_callback();
    assert_eq!(closed_calls(&instance), 1);

    // The host acknowledges, per the spec, with destroy.
    destroy(&mut instance);

    // -- editor 1: a late report must not be pinned on its successor -----
    // The host destroys editor 1 and opens editor 2; editor 1's close,
    // which raced that teardown in its runtime, lands only afterwards.
    open(&mut instance);
    destroy(&mut instance);
    open(&mut instance);
    user_closes(1);
    instance.call_on_main_thread_callback();
    assert_eq!(
        closed_calls(&instance),
        1,
        "a close report from a destroyed editor must not close its successor"
    );

    // -- editor 2, the live one, still reports its own close ---------------
    user_closes(2);
    instance.call_on_main_thread_callback();
    assert_eq!(
        closed_calls(&instance),
        2,
        "the live editor's close is reported"
    );
    destroy(&mut instance);

    // -- after destroy, with no editor at all, a report is dropped too -----
    open(&mut instance); // editor 3
    destroy(&mut instance);
    user_closes(3);
    instance.call_on_main_thread_callback();
    assert_eq!(closed_calls(&instance), 2);
}
