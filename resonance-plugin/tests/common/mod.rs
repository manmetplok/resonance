//! Shared test scaffolding: a host that drives a bridged plugin's real
//! `clap_plugin.process` and `clap_plugin_params.flush` across the C ABI.
//!
//! Most bridge behaviour can be tested through the main-thread extensions
//! alone (see `clap_bridge_params_state.rs`), but anything that only exists
//! *during* a process block — input events, output parameter events, gestures
//! — needs the real thing: clack-host building the `clap_process` struct, the
//! event lists and the audio buffers, and the bridge on the other side of the
//! ABI.
//!
//! Used by `midi_events.rs` (ba todo #1295) and `param_output.rs`
//! (ba todo #1293); ba todo #1341 wants the same harness for the
//! load-vs-editor CAS.

#![allow(dead_code)]

use clack_extensions::params::PluginParams;
use clack_host::prelude::*;
use clack_plugin::entry::SinglePluginEntry;

use resonance_plugin::{ClapBridge, ResonancePlugin};

pub struct TestHostShared;

impl SharedHandler<'_> for TestHostShared {
    fn request_restart(&self) {}
    fn request_process(&self) {}
    fn request_callback(&self) {}
}

pub struct TestHost;

impl HostHandlers for TestHost {
    type Shared<'a> = TestHostShared;
    type MainThread<'a> = ();
    type AudioProcessor<'a> = ();
}

/// Frames per `process()` block. 64 at 48 kHz is 1.33 ms, so a test can step
/// real time forward in small, realistic increments.
pub const FRAMES: usize = 64;
pub const SAMPLE_RATE: f64 = 48_000.0;

/// Drives one plugin instance through real `process()` / `flush()` calls.
pub struct ProcessHarness {
    instance: PluginInstance<TestHost>,
    processor: Option<StartedPluginAudioProcessor<TestHost>>,
    ports: AudioPorts,
    buffers: [[f32; FRAMES]; 2],
}

impl ProcessHarness {
    /// Load a bridged plugin and activate it, ready to process.
    pub fn new<P: ResonancePlugin>(bundle: &std::ffi::CStr, plugin_id: &std::ffi::CStr) -> Self {
        let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<P>>>(bundle)
            .expect("bundle entry init");
        let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();
        let mut instance = PluginInstance::<TestHost>::new(
            |_| TestHostShared,
            |_| (),
            &entry,
            plugin_id,
            &host_info,
        )
        .expect("plugin instantiation");

        let config = PluginAudioConfiguration {
            sample_rate: SAMPLE_RATE,
            min_frames_count: 32,
            max_frames_count: 8192,
        };
        let processor = instance
            .activate(|_, _| (), config)
            .expect("activation")
            .start_processing()
            .expect("start processing");

        Self {
            instance,
            processor: Some(processor),
            ports: AudioPorts::with_capacity(2, 1),
            buffers: [[0.0; FRAMES]; 2],
        }
    }

    /// Run one `process()` block carrying `events`, and return whatever the
    /// plugin pushed into the host's output event list.
    pub fn run(&mut self, events: &EventBuffer) -> EventBuffer {
        let mut outputs = self.ports.with_output_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_output_only(
                self.buffers.iter_mut().map(|b| b.as_mut_slice()),
            ),
        }]);
        let mut output_events = EventBuffer::new();

        self.processor
            .as_mut()
            .expect("processor")
            .process(
                &InputAudioBuffers::empty(),
                &mut outputs,
                &events.as_input(),
                &mut output_events.as_output(),
                None,
                None,
            )
            .expect("process");

        output_events
    }

    /// Run one block with no input events.
    pub fn run_empty(&mut self) -> EventBuffer {
        self.run(&EventBuffer::new())
    }

    /// Call `clap_plugin_params.flush` on the **active** plugin (the
    /// audio-processor flush), returning the output events.
    pub fn flush_active(&mut self, events: &EventBuffer) -> EventBuffer {
        let ext = self
            .instance
            .plugin_shared_handle()
            .get_extension::<PluginParams>()
            .expect("the bridge must expose the params extension");
        let mut output_events = EventBuffer::new();
        ext.flush_active(
            &mut self.processor.as_mut().expect("processor").plugin_handle(),
            &events.as_input(),
            &mut output_events.as_output(),
        );
        output_events
    }

    pub fn instance(&mut self) -> &mut PluginInstance<TestHost> {
        &mut self.instance
    }
}

impl Drop for ProcessHarness {
    fn drop(&mut self) {
        if let Some(processor) = self.processor.take() {
            self.instance.deactivate(processor.stop_processing());
        }
    }
}
