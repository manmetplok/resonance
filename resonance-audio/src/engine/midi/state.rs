//! Hardware MIDI state owned by the engine control thread.
//!
//! Carved out of [`super::super::thread::HandlerState`] so the handler
//! state doesn't carry a dozen MIDI-only fields. The control thread
//! holds a single `MidiHardwareState` and passes it (via the handler
//! state) into the live / hardware / outbound modules.

use std::collections::HashMap;

use crossbeam_channel::Sender;

use crate::midi_hardware::{LiveMidiEvent, MidiDeviceInfo, MidiInputRegistry, MidiOutputRegistry};
use crate::types::TrackId;

/// Aggregated hardware-MIDI bookkeeping for the engine control thread.
///
/// Owns the open midir connections (input + output registries), the
/// last-seen device lists (used to suppress redundant `MidiInputDevicesListed`
/// / `MidiOutputDevicesListed` events), and the playhead → outbound-note
/// state machine for routing timeline notes to hardware MIDI outputs.
pub struct MidiHardwareState {
    /// Hardware MIDI input registry. Owns one open midir connection
    /// per track configured for hardware input. The connection's
    /// callback runs on a midir-spawned thread and feeds
    /// [`LiveMidiEvent`]s into the engine thread via a bounded channel.
    pub midi_inputs: MidiInputRegistry,
    /// Hardware MIDI output registry. Refcounts midir output
    /// connections across tracks that share the same physical port.
    pub midi_outputs: MidiOutputRegistry,
    /// Notes currently sounding on hardware MIDI outputs from
    /// timeline playback. Keyed by `(track_id, note)`; value carries
    /// the note's end-sample plus the channel it was sent on (so a
    /// later channel change doesn't strand the stuck note).
    pub midi_outbound_held: HashMap<(TrackId, u8), (u64, u8)>,
    /// Last playhead seen by the timeline → output poll. The next
    /// poll iterates notes whose start/end fall in
    /// `(midi_outbound_last_playhead .. current_playhead]` and emits
    /// NoteOn/NoteOff for them.
    pub midi_outbound_last_playhead: u64,
    /// Last MIDI input device list sent to the GUI. Compared against
    /// the next enumeration so a steady-state poll (no plug events)
    /// doesn't trigger a redundant `MidiInputDevicesListed` round-trip.
    pub last_midi_input_devices: Vec<MidiDeviceInfo>,
    /// Same idea, output side.
    pub last_midi_output_devices: Vec<MidiDeviceInfo>,
}

impl MidiHardwareState {
    pub fn new(live_midi_tx: Sender<LiveMidiEvent>) -> Self {
        Self {
            midi_inputs: MidiInputRegistry::new(live_midi_tx),
            midi_outputs: MidiOutputRegistry::new(),
            midi_outbound_held: HashMap::new(),
            midi_outbound_last_playhead: 0,
            last_midi_input_devices: Vec::new(),
            last_midi_output_devices: Vec::new(),
        }
    }
}
