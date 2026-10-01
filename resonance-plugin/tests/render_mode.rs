//! CLAP `render` through the bridge: a host's `render.set(OFFLINE /
//! REALTIME)` reaches `ResonancePlugin::set_render_mode` — at once while
//! the plugin is inactive, and at the top of the next block while it is
//! active (the main thread cannot reach it then).

mod common;

use std::cell::RefCell;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use clack_extensions::render::{PluginRender, RenderMode};
use clack_host::prelude::*;
use clack_plugin::entry::SinglePluginEntry;

use common::{ProcessHarness, TestHost, TestHostShared};
use resonance_plugin::{
    ClapBridge, EventIterator, FloatParam, FloatRange, OutputBuffer, Param, ResonancePlugin,
    TempoInfo,
};

/// What the plugin was last told: 0 never, 1 realtime, 2 offline.
const NEVER: u8 = 0;
const REALTIME: u8 = 1;
const OFFLINE: u8 = 2;

thread_local! {
    static MODE: RefCell<Option<Arc<AtomicU8>>> = const { RefCell::new(None) };
}

fn told() -> u8 {
    MODE.with(|m| m.borrow().clone())
        .expect("plugin built")
        .load(Ordering::SeqCst)
}

struct ModePlugin {
    gain: FloatParam,
    mode: Arc<AtomicU8>,
}

impl ResonancePlugin for ModePlugin {
    const CLAP_ID: &'static str = "test.render-mode";
    const NAME: &'static str = "RenderMode";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static std::ffi::CStr] =
        &[resonance_plugin::features::AUDIO_EFFECT];
    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        let mode = Arc::new(AtomicU8::new(NEVER));
        // The bridge also builds a throwaway instance for its metadata;
        // the last one built on this thread is the live one.
        MODE.with(|m| *m.borrow_mut() = Some(mode.clone()));
        Self {
            gain: FloatParam::new(
                "gain",
                "Gain",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            mode,
        }
    }
    fn param_count(&self) -> usize {
        1
    }
    fn param(&self, _index: usize) -> &dyn Param {
        &self.gain
    }
    fn initialize(&mut self, _sample_rate: f32, _max_buffer_size: u32) -> bool {
        true
    }
    fn reset(&mut self) {}
    fn set_render_mode(&mut self, offline: bool) {
        self.mode
            .store(if offline { OFFLINE } else { REALTIME }, Ordering::SeqCst);
    }
    fn process(
        &mut self,
        _outputs: &mut [OutputBuffer<'_>],
        _frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
    }
}

const BUNDLE: &std::ffi::CStr = c"resonance-test-render-mode.clap";
const PLUGIN_ID: &std::ffi::CStr = c"test.render-mode";

fn render_ext(instance: &PluginInstance<TestHost>) -> PluginRender {
    instance
        .plugin_shared_handle()
        .get_extension::<PluginRender>()
        .expect("the bridge serves clap.render")
}

#[test]
fn an_inactive_plugin_is_told_at_once() {
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<ModePlugin>>>(BUNDLE)
        .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();
    let mut instance =
        PluginInstance::<TestHost>::new(|_| TestHostShared, |_| (), &entry, PLUGIN_ID, &host_info)
            .expect("plugin instantiation");
    let ext = render_ext(&instance);
    assert!(!ext.has_realtime_requirement(&mut instance.plugin_handle()));
    assert_eq!(told(), NEVER);

    ext.set(&mut instance.plugin_handle(), RenderMode::Offline)
        .expect("accepted");
    assert_eq!(told(), OFFLINE);
    ext.set(&mut instance.plugin_handle(), RenderMode::Realtime)
        .expect("accepted");
    assert_eq!(told(), REALTIME);
}

#[test]
fn an_active_plugin_is_told_at_the_top_of_the_next_block() {
    let mut harness = ProcessHarness::new::<ModePlugin>(BUNDLE, PLUGIN_ID);
    let ext = render_ext(harness.instance());
    ext.set(&mut harness.instance().plugin_handle(), RenderMode::Offline)
        .expect("accepted");
    assert_eq!(told(), NEVER, "the plugin is in the audio processor");
    harness.run_empty();
    assert_eq!(told(), OFFLINE);

    ext.set(
        &mut harness.instance().plugin_handle(),
        RenderMode::Realtime,
    )
    .expect("accepted");
    harness.run_empty();
    assert_eq!(told(), REALTIME);
}
