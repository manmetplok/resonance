//! The plugin behind the real CLAP C ABI, in-process: it instantiates,
//! activates, processes audio, and reports **zero latency at every
//! oversampling factor** — switched while active, through the same
//! parameter event a host automation lane, a preset recall and
//! `track.set_plugin_param` all send — without ever asking the host for
//! a restart (decision D3: the IIR oversampler has no fixed latency, so
//! there is nothing to report).

use std::sync::atomic::{AtomicBool, Ordering};

use clack_extensions::latency::{HostLatency, HostLatencyImpl, PluginLatency};
use clack_host::events::event_types::ParamValueEvent;
use clack_host::prelude::*;
use clack_host::utils::Cookie;
use clack_plugin::entry::SinglePluginEntry;

use resonance_color::ResonanceColor;
use resonance_dsp::OversampleFactor;
use resonance_plugin::{stable_hash, ClapBridge};

const SAMPLE_RATE: f32 = 48_000.0;
const FRAMES: usize = 512;

#[derive(Default)]
struct TestHostShared {
    restart_requested: AtomicBool,
    latency_changed: AtomicBool,
}

impl SharedHandler<'_> for TestHostShared {
    fn request_restart(&self) {
        self.restart_requested.store(true, Ordering::SeqCst)
    }
    fn request_process(&self) {}
    fn request_callback(&self) {}
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
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<ResonanceColor>>>(
        c"resonance-color-host.clap",
    )
    .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();
    PluginInstance::<TestHost>::new(
        |_| TestHostShared::default(),
        |shared| TestHostMainThread { shared },
        &entry,
        c"com.resonance.color",
        &host_info,
    )
    .expect("plugin instantiation")
}

fn audio_config() -> PluginAudioConfiguration {
    PluginAudioConfiguration {
        sample_rate: SAMPLE_RATE as f64,
        min_frames_count: 32,
        max_frames_count: 8192,
    }
}

fn query_latency(instance: &mut PluginInstance<TestHost>) -> u32 {
    let ext = instance
        .plugin_shared_handle()
        .get_extension::<PluginLatency>()
        .expect("the bridge must expose the latency extension");
    ext.get(&mut instance.plugin_handle())
}

fn param_event(id: &str, value: f64) -> ParamValueEvent {
    ParamValueEvent::new(
        0,
        ClapId::new(stable_hash(id)),
        Pckn::match_all(),
        value,
        Cookie::empty(),
    )
}

/// One block of a 220 Hz sine at −12 dBFS in, the left output back.
fn process_block(
    processor: &mut StartedPluginAudioProcessor<TestHost>,
    block: usize,
    events: &[ParamValueEvent],
) -> Vec<f32> {
    let mut input_ports = AudioPorts::with_capacity(2, 1);
    let mut output_ports = AudioPorts::with_capacity(2, 1);
    let mut in_left: Vec<f32> = (0..FRAMES)
        .map(|i| {
            let t = (block * FRAMES + i) as f32 / SAMPLE_RATE;
            0.25 * (std::f32::consts::TAU * 220.0 * t).sin()
        })
        .collect();
    let mut in_right = in_left.clone();
    let mut out_left = vec![0.0_f32; FRAMES];
    let mut out_right = vec![0.0_f32; FRAMES];

    let mut input_event_buffer = EventBuffer::new();
    for event in events {
        input_event_buffer.push(event);
    }
    let mut output_event_buffer = EventBuffer::new();
    {
        let input_events = input_event_buffer.as_input();
        let mut output_events = output_event_buffer.as_output();
        let input_audio = input_ports.with_input_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_input_only(
                [
                    InputChannel::variable(&mut in_left),
                    InputChannel::variable(&mut in_right),
                ]
                .into_iter(),
            ),
        }]);
        let mut output_audio = output_ports.with_output_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_output_only(
                [out_left.as_mut_slice(), out_right.as_mut_slice()].into_iter(),
            ),
        }]);
        processor
            .process(
                &input_audio,
                &mut output_audio,
                &input_events,
                &mut output_events,
                None,
                None,
            )
            .expect("process");
    }
    out_left
}

#[test]
fn every_oversampling_factor_reports_zero_latency_and_never_restarts() {
    let mut instance = instantiate();
    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");
    assert_eq!(query_latency(&mut instance), 0, "the default (2x) reports no latency");
    let mut processor = processor.start_processing().expect("start processing");

    let mut block = 0;
    for factor in [
        OversampleFactor::Off,
        OversampleFactor::X4,
        OversampleFactor::X2,
        OversampleFactor::Off,
    ] {
        let mut peak = 0.0f32;
        for i in 0..8 {
            let events = if i == 0 {
                vec![param_event("oversample", factor as i32 as f64)]
            } else {
                Vec::new()
            };
            let out = process_block(&mut processor, block, &events);
            block += 1;
            for s in &out {
                assert!(s.is_finite(), "{factor:?}: non-finite output");
                peak = peak.max(s.abs());
            }
        }
        assert!(peak > 0.1, "{factor:?}: the plugin passed (near) silence, peak {peak}");
        assert_eq!(query_latency(&mut instance), 0, "{factor:?} reports a latency");
        assert!(
            !instance
                .access_shared_handler(|h| &h.restart_requested)
                .load(Ordering::SeqCst),
            "{factor:?}: an oversampling change asked the host for a restart"
        );
    }
    instance.call_on_main_thread_callback();
    assert!(
        !instance
            .access_shared_handler(|h| &h.latency_changed)
            .load(Ordering::SeqCst),
        "the plugin told the host its latency changed"
    );

    let stopped = processor.stop_processing();
    instance.deactivate(stopped);

    // …and a fresh activation at each factor reads 0 as well.
    for factor in [OversampleFactor::Off, OversampleFactor::X2, OversampleFactor::X4] {
        let state = format!(r#"{{"params": {{"oversample": {}}}}}"#, factor as i32);
        let ext = instance
            .plugin_shared_handle()
            .get_extension::<clack_extensions::state::PluginState>()
            .expect("state extension");
        ext.load(&mut instance.plugin_handle(), &mut state.as_bytes())
            .expect("state load");
        let processor = instance
            .activate(|_, _| (), audio_config())
            .expect("activation");
        assert_eq!(query_latency(&mut instance), 0, "{factor:?} at activation");
        instance.deactivate(processor);
    }
}

/// Zero latency is also what the plugin object says on its own.
#[test]
fn the_plugin_object_reports_zero_latency_at_every_factor() {
    use resonance_plugin::ResonancePlugin;
    let mut plugin = ResonanceColor::new();
    for factor in [OversampleFactor::Off, OversampleFactor::X2, OversampleFactor::X4] {
        plugin.params.oversample.set_value(factor as i32);
        plugin.initialize(SAMPLE_RATE, 512);
        assert_eq!(plugin.latency_samples(), 0);
    }
}
