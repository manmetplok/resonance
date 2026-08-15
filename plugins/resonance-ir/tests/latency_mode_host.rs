//! A latency-mode change, driven end to end across the real CLAP ABI
//! (ba todo #1300, audit finding I1; host handle from ba todo #1296).
//!
//! `tests/latency_mode.rs` pins the plugin's own arithmetic. This file
//! pins the part that keeps plugin delay compensation honest, in the
//! order a host and this plugin actually perform it:
//!
//! 1. the host activates and reads `clap_plugin_latency.get()` — the
//!    figure it compensates every other track against;
//! 2. an impulse through `clap_plugin.process()` comes out *exactly* that
//!    many samples late, so the number is not a claim, it is the delay;
//! 3. a parameter event switches the mode while the plugin is active. The
//!    plugin cannot change its block size there (CLAP only allows the
//!    reported latency to move while deactivated, and new delay lines are
//!    an allocation), so it pushes the new figure through the host handle,
//!    which asks the host for a restart and a main-thread callback;
//! 4. the host runs the callback (`clap_host_latency.changed()`), then
//!    services the restart the way `resonance-audio`'s `clap_host` does —
//!    deactivate, reactivate, re-read the latency, republish PDC;
//! 5. after that cycle the reported latency is the new mode's, and an
//!    impulse comes out exactly that many samples late again.
//!
//! Step 5 is the acceptance criterion "PDC stays correct across a change":
//! the number the host compensates by and the delay the DSP imposes are
//! the same at both ends of the change.

use std::sync::atomic::{AtomicBool, Ordering};

use clack_extensions::latency::{HostLatency, HostLatencyImpl, PluginLatency};
use clack_host::events::event_types::ParamValueEvent;
use clack_host::prelude::*;
use clack_host::utils::Cookie;
use clack_plugin::entry::SinglePluginEntry;

use resonance_ir::dsp::{self, LatencyMode};
use resonance_ir::ResonanceIr;
use resonance_plugin::{stable_hash, ClapBridge};

const SAMPLE_RATE: f32 = 48_000.0;
/// Frames per `process()` call — comfortably longer than any block size
/// under test, so one call carries the whole impulse response.
const FRAMES: usize = 1024;

// ---------------------------------------------------------------------------
// A host that implements what a latency change needs
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestHostShared {
    restart_requested: AtomicBool,
    callback_requested: AtomicBool,
    latency_changed: AtomicBool,
}

impl SharedHandler<'_> for TestHostShared {
    fn request_restart(&self) {
        self.restart_requested.store(true, Ordering::SeqCst)
    }
    fn request_process(&self) {}
    fn request_callback(&self) {
        self.callback_requested.store(true, Ordering::SeqCst)
    }
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
    let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<ResonanceIr>>>(
        c"resonance-ir-latency-mode.clap",
    )
    .expect("bundle entry init");
    let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();

    PluginInstance::<TestHost>::new(
        |_| TestHostShared::default(),
        |shared| TestHostMainThread { shared },
        &entry,
        c"com.resonance.ir",
        &host_info,
    )
    .expect("plugin instantiation")
}

/// The activation config the real host uses (`clap_host/bundle.rs`).
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

fn take_flag(
    instance: &PluginInstance<TestHost>,
    pick: fn(&TestHostShared) -> &AtomicBool,
) -> bool {
    instance
        .access_shared_handler(pick)
        .swap(false, Ordering::SeqCst)
}

/// The mode parameter as a host writes it: a param-value event addressed
/// by the same id hash the bridge registered.
fn mode_event(mode: LatencyMode) -> ParamValueEvent {
    ParamValueEvent::new(
        0,
        ClapId::new(stable_hash("latency_mode")),
        Pckn::match_all(),
        mode.index() as f64,
        Cookie::empty(),
    )
}

/// Process one block, optionally with an impulse in the first sample and
/// a parameter event. Returns the left output channel.
///
/// With no IR loaded the plugin has no convolver, so the signal takes the
/// bypass-delay path — which exists precisely to keep the dry signal
/// aligned with the convolver's latency. Its delay is therefore the
/// plugin's latency, which is what makes this a measurement of the figure
/// the host compensates by.
fn process_block(
    processor: &mut StartedPluginAudioProcessor<TestHost>,
    impulse: bool,
    events: &[ParamValueEvent],
) -> Vec<f32> {
    let mut input_ports = AudioPorts::with_capacity(2, 1);
    let mut output_ports = AudioPorts::with_capacity(2, 1);

    let mut in_left = vec![0.0_f32; FRAMES];
    let mut in_right = vec![0.0_f32; FRAMES];
    if impulse {
        in_left[0] = 1.0;
        in_right[0] = 1.0;
    }
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

/// Send an impulse through the plugin and return how many samples late it
/// comes out.
fn measure_delay(processor: &mut StartedPluginAudioProcessor<TestHost>) -> usize {
    let out = process_block(processor, true, &[]);
    out.iter()
        .position(|s| s.abs() > 0.5)
        .expect("the impulse must come out within one block")
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

/// One test: the plugin instance is a single stateful object and every
/// step below depends on the one before it.
#[test]
fn a_latency_mode_change_reaches_the_host_and_the_dsp_agrees_with_it() {
    let normal = dsp::block_size_for(SAMPLE_RATE, LatencyMode::Normal);
    let tracking = dsp::block_size_for(SAMPLE_RATE, LatencyMode::Tracking);
    assert!(
        tracking < normal,
        "the modes must differ for this to prove anything"
    );

    let mut instance = instantiate();

    // -- 1. activation: the host reads the latency it will compensate by --
    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("activation");
    assert_eq!(
        query_latency(&mut instance) as usize,
        normal,
        "a fresh plugin reports the default mode's block size"
    );

    let mut processor = processor.start_processing().expect("start processing");

    // -- 2. …and that figure is the delay it really imposes ---------------
    assert_eq!(
        measure_delay(&mut processor),
        normal,
        "the reported latency must be the delay the audio actually takes"
    );

    // -- 3. the mode changes while the plugin is active -------------------
    // Same path a host automation lane, a preset recall and
    // `track.set_plugin_param` all take.
    let out = process_block(&mut processor, false, &[mode_event(LatencyMode::Tracking)]);
    assert!(
        out.iter().all(|s| s.abs() < 1e-6),
        "a mode change must not make noise on the block it arrives"
    );

    assert!(
        take_flag(&instance, |h| &h.restart_requested),
        "changing the block size while active must ask the host for a restart — \
         it is the only way CLAP lets the reported latency move"
    );
    assert!(
        take_flag(&instance, |h| &h.callback_requested),
        "…and for the main-thread callback that carries the notification"
    );
    assert_eq!(
        query_latency(&mut instance) as usize,
        tracking,
        "a query before the restart already reports the new figure"
    );

    // -- 4. the host runs the callback and services the restart -----------
    instance.call_on_main_thread_callback();
    assert!(
        take_flag(&instance, |h| &h.latency_changed),
        "the bridge must call clap_host_latency.changed() so the host re-reads and \
         recomputes plugin delay compensation"
    );

    let stopped = processor.stop_processing();
    instance.deactivate(stopped);
    let processor = instance
        .activate(|_, _| (), audio_config())
        .expect("re-activation");
    assert_eq!(
        query_latency(&mut instance) as usize,
        tracking,
        "the re-read after the restart is the new mode's block size — and it must \
         not have been reverted by the activation-time parameter sync"
    );

    // -- 5. PDC is still correct: reported == imposed ---------------------
    let mut processor = processor.start_processing().expect("restart processing");
    assert_eq!(
        measure_delay(&mut processor),
        tracking,
        "after the change the plugin must delay by exactly what it now reports"
    );

    // -- and a redundant selection does not cycle the plugin again --------
    let _ = process_block(&mut processor, false, &[mode_event(LatencyMode::Tracking)]);
    assert!(
        !take_flag(&instance, |h| &h.restart_requested),
        "selecting the mode that is already running must not restart anything"
    );

    let stopped = processor.stop_processing();
    instance.deactivate(stopped);
}
