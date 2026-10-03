//! `HostHandle::announce_param_change`: a param the plugin set itself (its
//! editor's kit browser, say — drums-plugin-rework.md §5.1) reaches the
//! host as one complete edit — gesture begin, value, gesture end — which
//! is what a host records as one undo step. Through the real C ABI:
//! `process()` and both `params.flush` paths.

mod common;

use std::cell::RefCell;
use std::sync::Arc;

use clack_extensions::params::PluginParams;
use clack_host::events::event_types::{
    ParamGestureBeginEvent, ParamGestureEndEvent, ParamValueEvent,
};
use clack_host::prelude::*;
use clack_plugin::entry::SinglePluginEntry;

use common::{ProcessHarness, TestHost, TestHostShared};
use resonance_plugin::{
    stable_hash, ClapBridge, EventIterator, FloatParam, FloatRange, HostHandle, IntParam, IntRange,
    OutputBuffer, Param, ResonancePlugin, TempoInfo,
};

thread_local! {
    /// The plugin's handle and its shared selector, as `set_host` saw
    /// them (each test builds its instance on its own thread).
    static LIVE: RefCell<Option<(Arc<HostHandle>, Arc<IntParam>)>> = const { RefCell::new(None) };
}

fn live() -> (Arc<HostHandle>, Arc<IntParam>) {
    LIVE.with(|l| l.borrow().clone()).expect("set_host ran")
}

struct AnnouncingPlugin {
    gain: FloatParam,
    /// Not automatable: set by the plugin's own browser, like the drums'
    /// `kit_select`.
    selector: Arc<IntParam>,
}

impl ResonancePlugin for AnnouncingPlugin {
    const CLAP_ID: &'static str = "test.param-announce";
    const NAME: &'static str = "ParamAnnounce";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static std::ffi::CStr] =
        &[resonance_plugin::features::AUDIO_EFFECT];
    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            gain: FloatParam::new(
                "gain",
                "Gain",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            selector: Arc::new(
                IntParam::new(
                    "selector",
                    "Selector",
                    -1,
                    IntRange::Linear { min: -1, max: 9 },
                )
                .not_automatable(),
            ),
        }
    }
    fn param_count(&self) -> usize {
        2
    }
    fn param(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.gain,
            _ => &*self.selector,
        }
    }
    fn initialize(&mut self, _sample_rate: f32, _max_buffer_size: u32) -> bool {
        true
    }
    fn reset(&mut self) {}
    fn set_host(&mut self, host: Arc<HostHandle>) {
        LIVE.with(|l| *l.borrow_mut() = Some((host, self.selector.clone())));
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

const BUNDLE: &std::ffi::CStr = c"resonance-test-param-announce.clap";
const PLUGIN_ID: &std::ffi::CStr = c"test.param-announce";

/// What came out, as `(kind, param id, value)`: kind is `b` / `v` / `e`.
fn decode(events: &EventBuffer) -> Vec<(char, u32, f64)> {
    let mut out = Vec::new();
    for event in events {
        if let Some(e) = event.as_event::<ParamGestureBeginEvent>() {
            out.push(('b', e.param_id().map_or(0, |id| id.get()), 0.0));
        } else if let Some(e) = event.as_event::<ParamValueEvent>() {
            out.push(('v', e.param_id().map_or(0, |id| id.get()), e.value()));
        } else if let Some(e) = event.as_event::<ParamGestureEndEvent>() {
            out.push(('e', e.param_id().map_or(0, |id| id.get()), 0.0));
        }
    }
    out
}

fn selector_edit(value: f64) -> Vec<(char, u32, f64)> {
    let id = stable_hash("selector");
    vec![('b', id, 0.0), ('v', id, value), ('e', id, 0.0)]
}

#[test]
fn an_announced_change_goes_out_of_the_next_block_as_one_gesture() {
    let mut harness = ProcessHarness::new::<AnnouncingPlugin>(BUNDLE, PLUGIN_ID);
    assert!(
        decode(&harness.run_empty()).is_empty(),
        "nothing announced yet"
    );

    let (host, selector) = live();
    selector.set_value(4);
    host.announce_param_change("selector");
    // Twice before the block: still one edit, at the latest value.
    selector.set_value(5);
    host.announce_param_change("selector");
    assert_eq!(decode(&harness.run_empty()), selector_edit(5.0));
    assert!(decode(&harness.run_empty()).is_empty(), "reported once");

    // The host's mirror agrees with the event.
    let ext = harness
        .instance()
        .plugin_shared_handle()
        .get_extension::<PluginParams>()
        .expect("params");
    let value = ext.get_value(
        &mut harness.instance().plugin_handle(),
        ClapId::new(stable_hash("selector")),
    );
    assert_eq!(value, Some(5.0));

    // An unknown id is ignored.
    host.announce_param_change("no-such-param");
    assert!(decode(&harness.run_empty()).is_empty());
}

/// A host with its transport stopped runs no block: the active flush
/// (which the announcement asked the host for) carries the edit instead.
#[test]
fn an_active_flush_carries_the_edit_when_no_block_runs() {
    let mut harness = ProcessHarness::new::<AnnouncingPlugin>(BUNDLE, PLUGIN_ID);
    let (host, selector) = live();
    selector.set_value(2);
    host.announce_param_change("selector");
    assert_eq!(
        decode(&harness.flush_active(&EventBuffer::new())),
        selector_edit(2.0)
    );
}

#[test]
fn an_inactive_flush_carries_the_edit() {
    let entry =
        PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<AnnouncingPlugin>>>(BUNDLE)
            .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();
    let mut instance =
        PluginInstance::<TestHost>::new(|_| TestHostShared, |_| (), &entry, PLUGIN_ID, &host_info)
            .expect("plugin instantiation");
    let (host, selector) = live();
    selector.set_value(7);
    host.announce_param_change("selector");

    let ext = instance
        .plugin_shared_handle()
        .get_extension::<PluginParams>()
        .expect("params");
    let mut output = EventBuffer::new();
    ext.flush(
        &mut instance.inactive_plugin_handle().expect("inactive"),
        &EventBuffer::new().as_input(),
        &mut output.as_output(),
    );
    assert_eq!(decode(&output), selector_edit(7.0));
}

// ---------------------------------------------------------------------------
// A renamed param announces under its pinned id (HOST-11)
// ---------------------------------------------------------------------------

/// [`AnnouncingPlugin`] after `selector` was renamed from `picker`.
struct RenamedAnnouncingPlugin(AnnouncingPlugin);

const PICKER_RENAME: &[resonance_plugin::ParamRename] = &[resonance_plugin::ParamRename {
    since_version: 1,
    from: "picker",
    to: "selector",
}];

impl ResonancePlugin for RenamedAnnouncingPlugin {
    const CLAP_ID: &'static str = "test.param-announce-renamed";
    const NAME: &'static str = "ParamAnnounceRenamed";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static std::ffi::CStr] =
        &[resonance_plugin::features::AUDIO_EFFECT];
    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self(AnnouncingPlugin::new())
    }
    fn param_count(&self) -> usize {
        self.0.param_count()
    }
    fn param(&self, index: usize) -> &dyn Param {
        self.0.param(index)
    }
    fn initialize(&mut self, sample_rate: f32, max_buffer_size: u32) -> bool {
        self.0.initialize(sample_rate, max_buffer_size)
    }
    fn reset(&mut self) {}
    fn set_host(&mut self, host: Arc<HostHandle>) {
        self.0.set_host(host);
    }
    fn param_renames(&self) -> &'static [resonance_plugin::ParamRename] {
        PICKER_RENAME
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

/// The plugin still announces by its current string id; the host is told
/// the id it has always known the param by — the pre-rename one — so an
/// undo step or automation lane recorded against it stays attached.
#[test]
fn a_renamed_param_announces_under_its_pinned_clap_id() {
    let mut harness = ProcessHarness::new::<RenamedAnnouncingPlugin>(
        c"resonance-test-param-announce-renamed.clap",
        c"test.param-announce-renamed",
    );
    let (host, selector) = live();
    selector.set_value(3);
    host.announce_param_change("selector");
    let id = stable_hash("picker");
    assert_eq!(
        decode(&harness.run_empty()),
        vec![('b', id, 0.0), ('v', id, 3.0), ('e', id, 0.0)]
    );
}
