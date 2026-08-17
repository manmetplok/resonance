//! Drive the CLAP bridge's `process()` across the real C ABI.
//!
//! Every other bridge test reaches the plugin through the *main-thread*
//! extensions (`params`, `state`, `latency`) or calls the `ResonancePlugin`
//! trait directly at the Rust level — `sidechain.rs`, for instance, calls
//! `plugin.process_with_key(..)` itself. Neither route touches
//! `clap_bridge/process.rs`, which is the audio path and holds the only
//! `unsafe` block in the crate: the `MaybeUninit` array of `OutputBuffer`
//! views that lets a multi-output-port plugin be called without allocating
//! on the audio thread.
//!
//! These tests build the real `clap_process` struct through clack-host —
//! input audio buffers, one buffer per declared output port, an event list
//! and a transport struct — and assert on what comes back out of the host's
//! buffers on the far side.
//!
//! The plugin under test writes everything it observed into its *own output
//! samples*, so there is no side channel and no shared global to make tests
//! interfere: if a value arrived wrong across the ABI, the audio says so.
//!
//! ba todo #1341.

mod common;

use clack_host::events::event_types::{
    NoteOffEvent, NoteOnEvent, ParamValueEvent, TransportEvent, TransportFlags,
};
use clack_host::events::{EventFlags, EventHeader, Match, Pckn};
use clack_host::prelude::*;
use clack_host::utils::{Cookie, FixedPoint};
use clack_plugin::entry::SinglePluginEntry;

use common::{ProcessHarness, TestHost, TestHostShared};
use resonance_plugin::{
    stable_hash, ClapBridge, EventIterator, FloatParam, FloatRange, NoteEvent, OutputBuffer,
    OutputPortSpec, Param, ResonancePlugin, TempoInfo,
};

// ---------------------------------------------------------------------------
// The plugin under test
// ---------------------------------------------------------------------------

/// Frames per block. Small enough that a test can spell out every sample it
/// cares about, large enough to place events at distinct timings.
const FRAMES: usize = 64;
const SAMPLE_RATE: f64 = 48_000.0;

/// Declared output ports. Three is deliberate: it puts more than one entry in
/// the `MaybeUninit` view array that `process()` builds behind its `unsafe`
/// block, so a bug in the initialize-then-`assume_init_mut` split shows up as
/// a wrong or missing port rather than being masked by a single-port layout.
const PORTS: usize = 3;

const PORT_MAIN: usize = 0;
const PORT_NOTES: usize = 1;
const PORT_META: usize = 2;

// Meta port (port 2) layout: the plugin reports what it was handed.
const META_FRAMES: usize = 0;
const META_GAIN: usize = 1;
const META_HAS_TEMPO: usize = 2;
const META_BPM: usize = 3;
const META_TIME_SIG_NUM: usize = 4;
const META_TIME_SIG_DEN: usize = 5;
const META_PLAYING: usize = 6;
const META_SONG_POS: usize = 7;

const GAIN_DEFAULT: f32 = 1.0;

/// An effect that multiplies its input by `gain`, and reports the note events
/// and transport it received through its two extra output ports.
struct AbiPlugin {
    gain: FloatParam,
}

impl ResonancePlugin for AbiPlugin {
    const CLAP_ID: &'static str = "test.abi-process";
    const NAME: &'static str = "AbiProcess";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static str] = &[];
    const INPUT_CHANNELS: Option<u32> = Some(2);
    const MIDI_INPUT: bool = true;

    fn output_layout(&self) -> Vec<OutputPortSpec> {
        vec![
            OutputPortSpec {
                name: "Main".into(),
                channel_count: 2,
            },
            OutputPortSpec {
                name: "Notes".into(),
                channel_count: 2,
            },
            OutputPortSpec {
                name: "Meta".into(),
                channel_count: 2,
            },
        ]
    }

    fn new() -> Self {
        Self {
            gain: FloatParam::new(
                "gain",
                "Gain",
                GAIN_DEFAULT,
                FloatRange::Linear {
                    min: 0.0,
                    max: 16.0,
                },
            ),
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

    fn process(
        &mut self,
        outputs: &mut [OutputBuffer<'_>],
        frames: usize,
        events: &mut EventIterator<'_>,
        tempo: Option<TempoInfo>,
    ) {
        let gain = self.gain.get_plain() as f32;

        // The bridge pre-fills port 0 with the incoming audio (the effect
        // contract), so this is a genuine in-place read-modify-write.
        {
            let main = &mut outputs[PORT_MAIN];
            for s in main.left[..frames].iter_mut() {
                *s *= gain;
            }
            for s in main.right[..frames].iter_mut() {
                *s *= gain;
            }
        }

        // Port 1 reports note events at their own timing: the note number on
        // the left (negated for note-off, offset for choke) and the velocity
        // on the right. Sample-accurate timings survive or they don't.
        {
            let notes = &mut outputs[PORT_NOTES];
            while let Some(event) = events.next_event() {
                match event {
                    NoteEvent::NoteOn {
                        note,
                        velocity,
                        timing,
                    } => {
                        notes.left[timing as usize] = f32::from(note);
                        notes.right[timing as usize] = velocity;
                    }
                    NoteEvent::NoteOff { note, timing } => {
                        notes.left[timing as usize] = -f32::from(note);
                    }
                    NoteEvent::Choke { note, timing } => {
                        notes.left[timing as usize] = -1000.0 - f32::from(note);
                    }
                }
            }
        }

        // Port 2 reports the scalar arguments of this block.
        {
            let meta = &mut outputs[PORT_META];
            meta.left[META_FRAMES] = frames as f32;
            meta.left[META_GAIN] = gain;
            match tempo {
                None => meta.left[META_HAS_TEMPO] = 0.0,
                Some(t) => {
                    meta.left[META_HAS_TEMPO] = 1.0;
                    meta.left[META_BPM] = t.bpm;
                    meta.left[META_TIME_SIG_NUM] = f32::from(t.time_sig_num);
                    meta.left[META_TIME_SIG_DEN] = f32::from(t.time_sig_den);
                    meta.left[META_PLAYING] = if t.playing { 1.0 } else { 0.0 };
                    meta.left[META_SONG_POS] = t.song_pos_beats as f32;
                }
            }
        }
    }
}

const BUNDLE: &std::ffi::CStr = c"resonance-test-abi-process.clap";
const PLUGIN_ID: &std::ffi::CStr = c"test.abi-process";

// ---------------------------------------------------------------------------
// A host that connects every declared port
// ---------------------------------------------------------------------------

/// `common::ProcessHarness` deliberately drives output-only, single-port
/// blocks — that is all `midi_events.rs` and `param_output.rs` need. This
/// plugin declares an input port and three output ports, so the audio-buffer
/// tests build their own `clap_process` with every port connected.
///
/// Kept local rather than folded into `common/mod.rs` so that file stays
/// byte-identical to the copy on `ba/todo-1293`, which is still in review.
struct AbiHarness {
    instance: PluginInstance<TestHost>,
    processor: Option<StartedPluginAudioProcessor<TestHost>>,
    input_ports: AudioPorts,
    output_ports: AudioPorts,
    input: [[f32; FRAMES]; 2],
    outputs: [[[f32; FRAMES]; 2]; PORTS],
}

impl AbiHarness {
    fn new() -> Self {
        let entry =
            PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<AbiPlugin>>>(BUNDLE)
                .expect("bundle entry init");
        let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();
        let mut instance = PluginInstance::<TestHost>::new(
            |_| TestHostShared,
            |_| (),
            &entry,
            PLUGIN_ID,
            &host_info,
        )
        .expect("plugin instantiation");

        let processor = instance
            .activate(
                |_, _| (),
                PluginAudioConfiguration {
                    sample_rate: SAMPLE_RATE,
                    min_frames_count: 32,
                    max_frames_count: 8192,
                },
            )
            .expect("activation")
            .start_processing()
            .expect("start processing");

        Self {
            instance,
            processor: Some(processor),
            input_ports: AudioPorts::with_capacity(2, 1),
            output_ports: AudioPorts::with_capacity(2 * PORTS, PORTS),
            input: [[0.0; FRAMES]; 2],
            outputs: [[[0.0; FRAMES]; 2]; PORTS],
        }
    }

    fn set_input(&mut self, f: impl Fn(usize) -> (f32, f32)) {
        for i in 0..FRAMES {
            let (l, r) = f(i);
            self.input[0][i] = l;
            self.input[1][i] = r;
        }
    }

    /// One real `process()` call with every port connected.
    fn run(&mut self, events: &EventBuffer, transport: Option<&TransportEvent>) {
        // Wipe the host-side output buffers so a port the plugin never wrote
        // is distinguishable from one it wrote zeros into.
        for port in self.outputs.iter_mut() {
            for channel in port.iter_mut() {
                channel.fill(f32::NAN);
            }
        }

        let inputs = self.input_ports.with_input_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_input_only(
                self.input.iter_mut().map(InputChannel::variable),
            ),
        }]);
        let mut outputs = self
            .output_ports
            .with_output_buffers(self.outputs.iter_mut().map(|port| AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only(
                    port.iter_mut().map(|c| c.as_mut_slice()),
                ),
            }));
        let mut output_events = EventBuffer::new();

        self.processor
            .as_mut()
            .expect("processor")
            .process(
                &inputs,
                &mut outputs,
                &events.as_input(),
                &mut output_events.as_output(),
                None,
                transport,
            )
            .expect("process");
    }

    fn port(&self, port: usize, channel: usize) -> &[f32] {
        &self.outputs[port][channel]
    }
}

impl Drop for AbiHarness {
    fn drop(&mut self) {
        if let Some(processor) = self.processor.take() {
            self.instance.deactivate(processor.stop_processing());
        }
    }
}

fn clap_id(id: &str) -> ClapId {
    ClapId::new(stable_hash(id))
}

fn param_event(time: u32, id: &str, value: f64) -> ParamValueEvent {
    ParamValueEvent::new(time, clap_id(id), Pckn::match_all(), value, Cookie::empty())
}

fn note_on(time: u32, key: u16, velocity: f64) -> NoteOnEvent {
    NoteOnEvent::new(time, Pckn::new(0u16, 0u16, key, Match::All), velocity)
}

fn note_off(time: u32, key: u16) -> NoteOffEvent {
    NoteOffEvent::new(time, Pckn::new(0u16, 0u16, key, Match::All), 0.0)
}

fn transport(bpm: f64, playing: bool) -> TransportEvent {
    TransportEvent {
        header: EventHeader::new_core(0, EventFlags::empty()),
        flags: if playing {
            TransportFlags::HAS_TEMPO
                | TransportFlags::HAS_TIME_SIGNATURE
                | TransportFlags::IS_PLAYING
        } else {
            TransportFlags::HAS_TEMPO | TransportFlags::HAS_TIME_SIGNATURE
        },
        song_pos_beats: FixedPoint::from_float(8.0),
        song_pos_seconds: FixedPoint::from_float(4.0),
        tempo: bpm,
        tempo_inc: 0.0,
        loop_start_beats: FixedPoint::from_int(0),
        loop_end_beats: FixedPoint::from_int(0),
        loop_start_seconds: FixedPoint::from_int(0),
        loop_end_seconds: FixedPoint::from_int(0),
        bar_start: FixedPoint::from_int(0),
        bar_number: 2,
        time_signature_numerator: 7,
        time_signature_denominator: 8,
    }
}

// ---------------------------------------------------------------------------
// Audio through the ABI
// ---------------------------------------------------------------------------

#[test]
fn input_audio_reaches_the_plugin_and_its_output_reaches_the_host() {
    let mut harness = AbiHarness::new();
    // A per-sample ramp, different on each channel, so a swapped or
    // short-copied channel cannot pass.
    harness.set_input(|i| (i as f32, -(i as f32) * 0.5));

    harness.run(&EventBuffer::new(), None);

    let left = harness.port(PORT_MAIN, 0);
    let right = harness.port(PORT_MAIN, 1);
    for i in 0..FRAMES {
        assert_eq!(
            left[i], i as f32,
            "left sample {i} did not survive the round trip at unity gain"
        );
        assert_eq!(
            right[i],
            -(i as f32) * 0.5,
            "right sample {i} did not survive the round trip at unity gain"
        );
    }
}

#[test]
fn a_param_event_in_the_block_is_applied_to_that_blocks_audio() {
    let mut harness = AbiHarness::new();
    harness.set_input(|_| (1.0, 1.0));

    let mut events = EventBuffer::new();
    events.push(&param_event(0, "gain", 3.0));
    harness.run(&events, None);

    assert_eq!(
        harness.port(PORT_META, 0)[META_GAIN],
        3.0,
        "the plugin must see the param event delivered in this block"
    );
    assert!(
        harness.port(PORT_MAIN, 0)[..FRAMES]
            .iter()
            .all(|s| *s == 3.0),
        "the block's audio must be scaled by the value the event carried"
    );

    // …and it sticks for the next block, which carries no events at all.
    harness.run(&EventBuffer::new(), None);
    assert!(
        harness.port(PORT_MAIN, 1)[..FRAMES]
            .iter()
            .all(|s| *s == 3.0),
        "the param value must persist into the following block"
    );
}

#[test]
fn every_declared_output_port_is_copied_back_to_the_host() {
    let mut harness = AbiHarness::new();
    harness.set_input(|_| (0.25, 0.25));

    let mut events = EventBuffer::new();
    events.push(&note_on(5, 64, 0.75));
    harness.run(&events, None);

    // Port 0: the processed audio.
    assert_eq!(harness.port(PORT_MAIN, 0)[0], 0.25);
    // Port 1: written only at the note's timing, zeroed everywhere else.
    assert_eq!(harness.port(PORT_NOTES, 0)[5], 64.0);
    assert_eq!(harness.port(PORT_NOTES, 0)[4], 0.0);
    // Port 2: the block's frame count.
    assert_eq!(harness.port(PORT_META, 0)[META_FRAMES], FRAMES as f32);

    // No port may be left as the NaN fill: that would mean the bridge never
    // copied its scratch back, i.e. the view array was built short.
    for port in 0..PORTS {
        for channel in 0..2 {
            assert!(
                harness.port(port, channel).iter().all(|s| !s.is_nan()),
                "port {port} channel {channel} was never written by the bridge"
            );
        }
    }
}

#[test]
fn note_events_arrive_with_their_sample_accurate_timing() {
    let mut harness = AbiHarness::new();

    let mut events = EventBuffer::new();
    events.push(&note_on(0, 60, 1.0));
    events.push(&note_on(17, 67, 0.5));
    events.push(&note_off(40, 60));
    harness.run(&events, None);

    let notes = harness.port(PORT_NOTES, 0);
    let velocities = harness.port(PORT_NOTES, 1);

    assert_eq!(notes[0], 60.0, "note-on at frame 0");
    assert_eq!(velocities[0], 1.0);
    assert_eq!(notes[17], 67.0, "note-on at frame 17");
    assert_eq!(velocities[17], 0.5);
    assert_eq!(notes[40], -60.0, "note-off at frame 40");

    // Nothing anywhere else.
    for (i, sample) in notes.iter().enumerate() {
        if i != 0 && i != 17 && i != 40 {
            assert_eq!(*sample, 0.0, "frame {i} must carry no note");
        }
    }
}

#[test]
fn the_transport_struct_reaches_the_plugin_as_tempo_info() {
    let mut harness = AbiHarness::new();

    harness.run(&EventBuffer::new(), None);
    assert_eq!(
        harness.port(PORT_META, 0)[META_HAS_TEMPO],
        0.0,
        "a free-running host passes no transport, so the plugin gets None"
    );

    let t = transport(132.0, true);
    harness.run(&EventBuffer::new(), Some(&t));
    let meta = harness.port(PORT_META, 0);
    assert_eq!(meta[META_HAS_TEMPO], 1.0);
    assert_eq!(meta[META_BPM], 132.0);
    assert_eq!(meta[META_TIME_SIG_NUM], 7.0);
    assert_eq!(meta[META_TIME_SIG_DEN], 8.0);
    assert_eq!(meta[META_PLAYING], 1.0);
    assert_eq!(meta[META_SONG_POS], 8.0);
}

#[test]
fn a_transport_without_the_tempo_flag_is_reported_as_no_tempo() {
    let mut harness = AbiHarness::new();

    let mut t = transport(120.0, true);
    t.flags = TransportFlags::IS_PLAYING;
    harness.run(&EventBuffer::new(), Some(&t));

    assert_eq!(
        harness.port(PORT_META, 0)[META_HAS_TEMPO],
        0.0,
        "the bridge must not invent a tempo the host did not flag as valid"
    );
}

#[test]
fn a_host_that_connects_fewer_output_ports_than_declared_is_survivable() {
    // `ProcessHarness` connects a single stereo output port. This plugin
    // declares three, so ports 1 and 2 have nowhere to be copied — the
    // bridge must skip them and still deliver port 0, not panic or write
    // through a dangling port.
    let mut harness = ProcessHarness::new::<AbiPlugin>(BUNDLE, PLUGIN_ID);

    let mut events = EventBuffer::new();
    events.push(&param_event(0, "gain", 2.0));
    let output_events = harness.run(&events);

    assert_eq!(
        output_events.len(),
        0,
        "this plugin reports no parameter changes of its own"
    );

    // Still alive on the next block, which is the real assertion.
    harness.run_empty();
}
