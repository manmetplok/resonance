//! A size the editor refuses reaches the host as a refusal, with the
//! editor's reason in the host's log.
//!
//! Before clack 0.2 (#102) `clap_plugin_gui.set_size` answered `true`
//! whatever the plugin's `set_size` returned, so a host resizing an editor
//! whose window had already gone was told it worked. Now the bridge's
//! `Err` becomes `false`, and clack logs it through `clap_host_log`; the
//! bridge makes that `Err` carry the editor's own
//! [`SizeRefused`] reason rather than a generic string.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use clack_extensions::gui::{GuiApiType, GuiConfiguration, GuiError, GuiSize, PluginGui};
use clack_extensions::log::{HostLog, HostLogImpl, LogSeverity};
use clack_host::prelude::*;
use clack_plugin::entry::SinglePluginEntry;

use resonance_plugin::gui::{EditorFactory, PluginEditor, SizeRefused};
use resonance_plugin::{
    ClapBridge, EventIterator, OutputBuffer, Param, ResonancePlugin, TempoInfo,
};

/// Widest size the fake editor takes.
const MAX_WIDTH: u32 = 1000;
const REASON: &str = "wider than the screen";

struct PickyEditor;

impl PluginEditor for PickyEditor {
    fn show(&mut self) {}
    fn hide(&mut self) {}
    fn size(&self) -> (u32, u32) {
        (400, 300)
    }
    fn set_size(&mut self, width: u32, _height: u32) -> Result<(), SizeRefused> {
        if width > MAX_WIDTH {
            Err(SizeRefused(REASON.into()))
        } else {
            Ok(())
        }
    }
    fn can_resize(&self) -> bool {
        true
    }
}

struct PickyFactory;

impl EditorFactory for PickyFactory {
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
            .then(|| Box::new(PickyEditor) as Box<dyn PluginEditor>)
    }
}

struct SizedEffect;

impl ResonancePlugin for SizedEffect {
    const CLAP_ID: &'static str = "test.editor-set-size";
    const NAME: &'static str = "EditorSetSize";
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
    fn param(&self, _index: usize) -> &dyn Param {
        unreachable!("test plugin declares zero params")
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
        Some(Arc::new(PickyFactory))
    }
}

#[derive(Default)]
struct Shared {
    log: Mutex<Vec<(LogSeverity, String)>>,
    callback_requested: AtomicBool,
}

impl SharedHandler<'_> for Shared {
    fn request_restart(&self) {}
    fn request_process(&self) {}
    fn request_callback(&self) {
        self.callback_requested.store(true, Ordering::SeqCst);
    }
}

impl HostLogImpl for Shared {
    fn log(&self, severity: LogSeverity, message: &str) {
        self.log.lock().unwrap().push((severity, message.to_owned()));
    }
}

struct Host;

impl HostHandlers for Host {
    type Shared<'a> = Shared;
    type MainThread<'a> = ();
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        builder.register::<HostLog>();
    }
}

fn logged(instance: &PluginInstance<Host>) -> Vec<String> {
    instance.access_shared_handler(|h| {
        h.log.lock().unwrap().iter().map(|(_, m)| m.clone()).collect()
    })
}

#[test]
fn a_refused_size_is_a_refusal_with_the_editors_reason_in_the_host_log() {
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<SizedEffect>>>(
        c"resonance-test-editor-set-size.clap",
    )
    .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();
    let mut instance = PluginInstance::<Host>::new(
        |_| Shared::default(),
        |_| (),
        &entry,
        c"test.editor-set-size",
        &host_info,
    )
    .expect("plugin instantiation");
    let gui = instance
        .plugin_shared_handle()
        .get_extension::<PluginGui>()
        .expect("gui extension");

    // No editor yet: nothing to resize.
    assert_eq!(
        gui.set_size(&instance.plugin_handle(), GuiSize { width: 500, height: 300 }),
        Err(GuiError::SetSizeError),
        "set_size with no editor open must fail"
    );

    gui.create(
        &instance.plugin_handle(),
        GuiConfiguration {
            api_type: GuiApiType::WAYLAND,
            is_floating: true,
        },
    )
    .expect("gui create");
    let before = logged(&instance).len();

    let fits = GuiSize { width: 800, height: 600 };
    assert_eq!(gui.set_size(&instance.plugin_handle(), fits), Ok(()));
    assert_eq!(logged(&instance).len(), before, "an accepted size logs nothing");

    let too_wide = GuiSize { width: MAX_WIDTH + 1, height: 600 };
    assert_eq!(
        gui.set_size(&instance.plugin_handle(), too_wide),
        Err(GuiError::SetSizeError),
        "a size the editor refuses must reach the host as a refusal"
    );
    let log = logged(&instance);
    assert_eq!(log.len(), before + 1, "the refusal is logged once: {log:?}");
    assert!(
        log.last().unwrap().contains(REASON),
        "the host log must carry the editor's reason, got {:?}",
        log.last()
    );

    gui.destroy(&instance.plugin_handle());
}
