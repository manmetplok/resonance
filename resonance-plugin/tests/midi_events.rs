//! Host-side tests for MIDI CC, aftertouch and pitch bend reaching a plugin
//! (ba todo #1295).
//!
//! Controller events only exist inside a `process()` call, so unlike the
//! other bridge tests these drive the plugin's real `clap_plugin.process`
//! across the C ABI: clack-host builds the `clap_process` struct, the event
//! list and the audio buffers, and the bridge decodes what arrives. The
//! plugin records what it saw, and the assertions are on that.
//!
//! The reason this needs an ABI test rather than a unit test is the part a
//! unit test cannot see: a host will not send a control change at all unless
//! the *note port* says it speaks the MIDI dialect, and the CLAP dialect has
//! no control change to send. Port declaration and event decoding have to
//! agree, and they live in different files.
//!
//! The harness (`ProcessHarness`) is reusable — ba todo #1341 wants to drive
//! `process()` through the ABI for the load-vs-editor CAS and can lift it.

use std::sync::{Mutex, OnceLock};

use clack_extensions::note_ports::{NoteDialect, NotePortInfoBuffer, PluginNotePorts};
use clack_host::events::event_types::{MidiEvent, NoteExpressionEvent, NoteOnEvent};
use clack_host::events::Match;
use clack_host::prelude::*;
use clack_plugin::entry::SinglePluginEntry;
use clack_plugin::events::event_types::NoteExpressionType;

use resonance_plugin::{
    ClapBridge, ControlEvent, EventIterator, NoteEvent, OutputBuffer, Param, PluginEvent,
    ResonancePlugin, TempoInfo,
};

// ---------------------------------------------------------------------------
// Test plugin: records every event the bridge hands it
// ---------------------------------------------------------------------------

/// Everything `process()` saw, in delivery order.
static SEEN: Mutex<Vec<PluginEvent>> = Mutex::new(Vec::new());

/// Set once per process call: what `next_event()` (the notes-only accessor
/// every existing plugin uses) yields for the same block.
static SEEN_NOTES_ONLY: OnceLock<Mutex<Vec<NoteEvent>>> = OnceLock::new();

fn seen_notes_only() -> &'static Mutex<Vec<NoteEvent>> {
    SEEN_NOTES_ONLY.get_or_init(|| Mutex::new(Vec::new()))
}

fn no_param(_: usize) -> &'static dyn Param {
    unreachable!("test plugin declares zero params")
}

struct MidiProbe;

impl ResonancePlugin for MidiProbe {
    const CLAP_ID: &'static str = "test.midi-probe";
    const NAME: &'static str = "MidiProbe";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static str] = &["instrument"];
    const INPUT_CHANNELS: Option<u32> = None;
    const MIDI_INPUT: bool = true;

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
        outputs: &mut [OutputBuffer<'_>],
        frames: usize,
        events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        let mut seen = SEEN.lock().unwrap();
        while let Some(event) = events.next_any() {
            seen.push(event);
        }
        // Write silence, so the block is a legitimate process call.
        for out in outputs.iter_mut() {
            out.left[..frames].fill(0.0);
            out.right[..frames].fill(0.0);
        }
    }
}

/// Same plugin, but draining through the legacy notes-only accessor. Proves
/// `next_event()` still behaves exactly as it did — controllers are skipped,
/// not surfaced as something a `match` on `NoteEvent` would have to handle.
struct NotesOnlyProbe;

impl ResonancePlugin for NotesOnlyProbe {
    const CLAP_ID: &'static str = "test.notes-only-probe";
    const NAME: &'static str = "NotesOnlyProbe";
    const VENDOR: &'static str = "test";
    const VERSION: &'static str = "0.0.0";
    const DESCRIPTION: &'static str = "";
    const FEATURES: &'static [&'static str] = &["instrument"];
    const INPUT_CHANNELS: Option<u32> = None;
    const MIDI_INPUT: bool = true;

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
        outputs: &mut [OutputBuffer<'_>],
        frames: usize,
        events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        let mut seen = seen_notes_only().lock().unwrap();
        while let Some(event) = events.next_event() {
            seen.push(event);
        }
        for out in outputs.iter_mut() {
            out.left[..frames].fill(0.0);
            out.right[..frames].fill(0.0);
        }
    }
}

// ---------------------------------------------------------------------------
// Minimal clack host + a process harness
// ---------------------------------------------------------------------------

struct TestHostShared;

impl SharedHandler<'_> for TestHostShared {
    fn request_restart(&self) {}
    fn request_process(&self) {}
    fn request_callback(&self) {}
}

struct TestHost;

impl HostHandlers for TestHost {
    type Shared<'a> = TestHostShared;
    type MainThread<'a> = ();
    type AudioProcessor<'a> = ();
}

const FRAMES: usize = 64;

/// Drives one plugin instance through real `process()` calls.
///
/// Owns the output buffers and the port scratch so a test can call
/// [`run`](Self::run) repeatedly with different event lists.
struct ProcessHarness {
    instance: PluginInstance<TestHost>,
    processor: Option<StartedPluginAudioProcessor<TestHost>>,
    ports: AudioPorts,
    buffers: [[f32; FRAMES]; 2],
}

impl ProcessHarness {
    fn new<P: ResonancePlugin>(bundle: &std::ffi::CStr, plugin_id: &std::ffi::CStr) -> Self {
        let entry = PluginEntry::load_from_clack::<SinglePluginEntry<ClapBridge<P>>>(bundle)
            .expect("bundle entry init");
        let host_info = HostInfo::new("test-host", "test", "https://example.com", "0.0.0").unwrap();
        let mut instance =
            PluginInstance::<TestHost>::new(|_| TestHostShared, |_| (), &entry, plugin_id, &host_info)
                .expect("plugin instantiation");

        let config = PluginAudioConfiguration {
            sample_rate: 48_000.0,
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

    /// Run one `process()` block carrying `events`.
    fn run(&mut self, events: &EventBuffer) {
        let mut outputs = self
            .ports
            .with_output_buffers([AudioPortBuffer {
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
    }

    fn plugin_instance(&mut self) -> &mut PluginInstance<TestHost> {
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

// ---------------------------------------------------------------------------
// Event builders
// ---------------------------------------------------------------------------

/// Raw MIDI 1.0 control change.
fn cc(time: u32, channel: u8, controller: u8, value: u8) -> MidiEvent {
    MidiEvent::new(time, 0, [0xb0 | channel, controller, value])
}

/// Raw MIDI 1.0 pitch bend, 14-bit LSB-first.
fn pitch_bend(time: u32, channel: u8, raw: u16) -> MidiEvent {
    MidiEvent::new(
        time,
        0,
        [0xe0 | channel, (raw & 0x7f) as u8, (raw >> 7) as u8],
    )
}

fn take_seen() -> Vec<PluginEvent> {
    std::mem::take(&mut *SEEN.lock().unwrap())
}

fn controls(events: &[PluginEvent]) -> Vec<ControlEvent> {
    events.iter().filter_map(|e| e.as_control()).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The whole controller surface in one block, because `SEEN` is process-wide
/// state that separate tests would race on. Each section is one finding.
#[test]
fn controller_events_reach_the_plugin_through_process() {
    let mut harness = ProcessHarness::new::<MidiProbe>(c"resonance-test-midi.clap", c"test.midi-probe");

    // -- the note port must advertise MIDI, or no host would send any of
    //    this: the CLAP dialect has no control change at all ---------------
    {
        let instance = harness.plugin_instance();
        let ext = instance
            .plugin_shared_handle()
            .get_extension::<PluginNotePorts>()
            .expect("a MIDI_INPUT plugin must expose the note-ports extension");
        let mut buffer = NotePortInfoBuffer::new();
        let info = ext
            .get(&mut instance.plugin_handle(), 0, true, &mut buffer)
            .expect("one note input port");
        assert!(
            info.supported_dialects.supports(NoteDialect::Midi),
            "the note port must accept the MIDI dialect"
        );
        assert!(
            info.supported_dialects.supports(NoteDialect::Clap),
            "…without dropping the CLAP dialect"
        );
        assert_eq!(
            info.preferred_dialect,
            Some(NoteDialect::Clap),
            "notes should still arrive as CLAP events"
        );
    }

    // -- CC, channel pressure, poly pressure, pitch bend -------------------
    let mut events = EventBuffer::new();
    // Mod wheel (CC 1) fully up on channel 0.
    events.push(&cc(0, 0, 1, 127));
    // Sustain (CC 64) off on channel 3.
    events.push(&cc(1, 3, 64, 0));
    // Channel pressure, mid travel.
    events.push(&MidiEvent::new(2, 0, [0xd0, 64, 0]));
    // Poly key pressure on note 60.
    events.push(&MidiEvent::new(3, 0, [0xa0, 60, 127]));
    // Pitch bend: centre, then hard down, then hard up.
    events.push(&pitch_bend(4, 0, 8192));
    events.push(&pitch_bend(5, 0, 0));
    events.push(&pitch_bend(6, 0, 16383));
    harness.run(&events);

    let seen = take_seen();
    let got = controls(&seen);
    assert_eq!(
        got.len(),
        7,
        "every controller message must reach the plugin, got {got:?}"
    );

    assert_eq!(
        got[0],
        ControlEvent::ControlChange {
            channel: 0,
            controller: 1,
            value: 1.0,
            timing: 0,
        }
    );
    assert_eq!(
        got[1],
        ControlEvent::ControlChange {
            channel: 3,
            controller: 64,
            value: 0.0,
            timing: 1,
        },
        "the channel nibble and the CC number must survive decoding"
    );
    assert_eq!(
        got[2],
        ControlEvent::ChannelPressure {
            channel: 0,
            pressure: 64.0 / 127.0,
            timing: 2,
        }
    );
    assert_eq!(
        got[3],
        ControlEvent::PolyPressure {
            channel: 0,
            note: 60,
            pressure: 1.0,
            timing: 3,
        }
    );
    assert_eq!(
        got[4],
        ControlEvent::PitchBend {
            channel: 0,
            value: 0.0,
            timing: 4,
        },
        "8192 is centre and must decode to exactly 0.0"
    );
    assert_eq!(
        got[5],
        ControlEvent::PitchBend {
            channel: 0,
            value: -1.0,
            timing: 5,
        }
    );
    assert_eq!(
        got[6],
        ControlEvent::PitchBend {
            channel: 0,
            value: 1.0,
            timing: 6,
        },
        "a full upward bend must reach 1.0, not 0.9998"
    );

    // -- CLAP-native poly aftertouch (a note expression) -------------------
    let mut events = EventBuffer::new();
    events.push(&NoteExpressionEvent::new(
        8,
        Pckn::new(0u16, 1u16, 72u16, Match::All),
        NoteExpressionType::Pressure,
        0.5,
    ));
    // A note expression the bridge does not map must not turn into anything.
    events.push(&NoteExpressionEvent::new(
        9,
        Pckn::new(0u16, 1u16, 72u16, Match::All),
        NoteExpressionType::Pan,
        0.25,
    ));
    harness.run(&events);

    let seen = take_seen();
    let got = controls(&seen);
    assert_eq!(
        got,
        vec![ControlEvent::PolyPressure {
            channel: 1,
            note: 72,
            pressure: 0.5,
            timing: 8,
        }],
        "a CLAP-dialect host's per-note pressure must land on the same event"
    );

    // -- notes and controllers arrive in one ordered stream ----------------
    let mut events = EventBuffer::new();
    events.push(&cc(0, 0, 1, 64));
    events.push(&NoteOnEvent::new(
        0,
        Pckn::new(0u16, 0u16, 60u16, Match::All),
        0.75,
    ));
    events.push(&cc(32, 0, 1, 0));
    harness.run(&events);

    let seen = take_seen();
    assert_eq!(seen.len(), 3);
    assert!(matches!(seen[0], PluginEvent::Control(_)));
    assert_eq!(
        seen[1].as_note(),
        Some(NoteEvent::NoteOn {
            note: 60,
            velocity: 0.75,
            timing: 0,
        }),
        "notes must still arrive, unchanged, alongside the controllers"
    );
    assert_eq!(seen[2].timing(), 32);
    assert!(
        matches!(seen[2], PluginEvent::Control(_)),
        "host order must be preserved, not notes-then-controllers"
    );

    // -- unhandled MIDI is dropped, not mangled ----------------------------
    let mut events = EventBuffer::new();
    // Program change, and a MIDI note-on: notes come through the CLAP
    // dialect, so decoding them from MIDI too would double-trigger.
    events.push(&MidiEvent::new(0, 0, [0xc0, 5, 0]));
    events.push(&MidiEvent::new(1, 0, [0x90, 60, 100]));
    harness.run(&events);
    assert!(
        take_seen().is_empty(),
        "the bridge must ignore MIDI messages it does not map"
    );
}

/// The accessor every existing plugin uses keeps its old meaning: note
/// events only, with controllers skipped rather than injected into a stream
/// a `match NoteEvent { .. }` could not handle.
#[test]
fn the_notes_only_accessor_still_sees_exactly_the_notes() {
    let mut harness =
        ProcessHarness::new::<NotesOnlyProbe>(c"resonance-test-notes-only.clap", c"test.notes-only-probe");

    let mut events = EventBuffer::new();
    events.push(&cc(0, 0, 1, 127));
    events.push(&NoteOnEvent::new(
        1,
        Pckn::new(0u16, 0u16, 64u16, Match::All),
        1.0,
    ));
    events.push(&pitch_bend(2, 0, 0));
    events.push(&NoteOnEvent::new(
        3,
        Pckn::new(0u16, 0u16, 67u16, Match::All),
        0.5,
    ));
    harness.run(&events);

    let seen = std::mem::take(&mut *seen_notes_only().lock().unwrap());
    assert_eq!(
        seen,
        vec![
            NoteEvent::NoteOn {
                note: 64,
                velocity: 1.0,
                timing: 1,
            },
            NoteEvent::NoteOn {
                note: 67,
                velocity: 0.5,
                timing: 3,
            },
        ]
    );
}
