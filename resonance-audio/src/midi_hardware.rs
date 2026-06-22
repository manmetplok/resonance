//! Hardware MIDI device enumeration and per-track input/output
//! connection management.
//!
//! Threading model:
//! - The engine control thread owns the registries and is the only
//!   place that calls [`midir`] APIs (open/close connections, send
//!   notes). This keeps midir's per-platform mutexes (ALSA seq /
//!   CoreMIDI / WinMM) far away from the audio callback.
//! - Each opened input port spawns its own thread inside `midir`,
//!   which drives the closure registered with `connect()`. That
//!   closure parses the incoming MIDI bytes, applies the channel
//!   filter, and pushes a [`LiveMidiEvent`] into a bounded crossbeam
//!   channel. The engine control thread drains that channel each
//!   iteration and dispatches the event into the existing
//!   `handle_send_note_on` / `handle_send_note_off` paths.
//! - Output sends happen on the engine control thread only.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use crossbeam_channel::Sender;
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

use crate::types::TrackId;

/// A hardware MIDI port the user can pick from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiDeviceInfo {
    pub name: String,
}

impl std::fmt::Display for MidiDeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// Hardware MIDI input drained from the midir-spawned thread on the
/// engine control thread. Carries the wall-clock instant at which the
/// midir callback fired so the recorder can compensate for the
/// engine-thread drain delay (~16 ms) and write the note at its
/// actual press time rather than at processing time.
#[derive(Debug, Clone)]
pub enum LiveMidiEvent {
    /// Hardware MIDI input arrived on a track's configured input port.
    /// Drained on the engine control thread and routed to:
    /// (1) the track's instrument plugin via `queue_note_on`,
    /// (2) the track's record clip if armed and transport is playing,
    /// (3) the track's MIDI output device if configured (Thru).
    InboundNoteOn {
        track_id: TrackId,
        note: u8,
        velocity: f32,
        arrival: std::time::Instant,
    },
    InboundNoteOff {
        track_id: TrackId,
        note: u8,
        arrival: std::time::Instant,
    },
}

/// Enumerate currently-available MIDI input devices.
pub fn enumerate_midi_inputs() -> Vec<MidiDeviceInfo> {
    let input = match MidiInput::new("resonance-enumerate-in") {
        Ok(i) => i,
        Err(_) => return Vec::new(),
    };
    input
        .ports()
        .iter()
        .filter_map(|p| input.port_name(p).ok())
        .map(|name| MidiDeviceInfo { name })
        .collect()
}

/// Enumerate currently-available MIDI output devices.
pub fn enumerate_midi_outputs() -> Vec<MidiDeviceInfo> {
    let output = match MidiOutput::new("resonance-enumerate-out") {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    output
        .ports()
        .iter()
        .filter_map(|p| output.port_name(p).ok())
        .map(|name| MidiDeviceInfo { name })
        .collect()
}

/// Encodes the channel filter as either omni (`u8::MAX`) or a
/// specific 0-indexed channel (0..=15). Stored in an atomic so that
/// changing the filter doesn't require restarting the connection.
const CHANNEL_OMNI: u8 = u8::MAX;

struct ActiveInputConn {
    device_name: String,
    _conn: MidiInputConnection<()>,
    channel_filter: Arc<AtomicU8>,
}

/// Per-track hardware MIDI input registry. Owns one open
/// [`MidiInputConnection`] per track, plus a "pending" set for
/// tracks whose configured device isn't currently plugged in.
pub struct MidiInputRegistry {
    connections: HashMap<TrackId, ActiveInputConn>,
    pending: HashMap<TrackId, (String, Option<u8>)>,
    tx: Sender<LiveMidiEvent>,
}

impl MidiInputRegistry {
    pub fn new(tx: Sender<LiveMidiEvent>) -> Self {
        Self {
            connections: HashMap::new(),
            pending: HashMap::new(),
            tx,
        }
    }

    /// Set a track's MIDI input source. `device_name = None` removes
    /// any existing connection and clears any pending request. If the
    /// requested device isn't currently present the request is stored
    /// as pending and reconciled on the next [`Self::reconcile`] call.
    pub fn set_track_input(
        &mut self,
        track_id: TrackId,
        device_name: Option<String>,
        channel_filter: Option<u8>,
    ) -> Result<(), String> {
        // Live update: if a connection already exists for this track
        // and only the channel filter changed, swap the atomic in
        // place rather than re-opening.
        if let Some(active) = self.connections.get(&track_id) {
            if Some(&active.device_name) == device_name.as_ref() {
                active
                    .channel_filter
                    .store(encode_channel_filter(channel_filter), Ordering::Relaxed);
                return Ok(());
            }
        }

        // Drop any previous connection on this track.
        self.connections.remove(&track_id);
        self.pending.remove(&track_id);

        let Some(name) = device_name else {
            return Ok(());
        };

        // Try to open the requested device immediately. If it isn't
        // present, store the desire as pending and surface no error
        // — the user gets feedback through the picker (the
        // configured-but-missing italic style).
        match open_input(&name, track_id, channel_filter, self.tx.clone()) {
            Ok(active) => {
                self.connections.insert(track_id, active);
                Ok(())
            }
            Err(_) => {
                self.pending.insert(track_id, (name, channel_filter));
                Ok(())
            }
        }
    }

    /// Drop any connection associated with a removed track.
    pub fn remove_track(&mut self, track_id: TrackId) {
        self.connections.remove(&track_id);
        self.pending.remove(&track_id);
    }

    /// Walk the pending set and try to open connections for any
    /// devices that have just appeared. Called after every
    /// enumeration so a freshly plugged-in controller starts working
    /// without the user touching the picker.
    pub fn reconcile(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let pending: Vec<(TrackId, String, Option<u8>)> = self
            .pending
            .iter()
            .map(|(k, (n, c))| (*k, n.clone(), *c))
            .collect();
        for (track_id, name, channel) in pending {
            if let Ok(active) = open_input(&name, track_id, channel, self.tx.clone()) {
                self.connections.insert(track_id, active);
                self.pending.remove(&track_id);
            }
        }
    }
}

/// Try to open the named MIDI input port and wire up the message
/// callback. Used both by `set_track_input` and by `reconcile`.
fn open_input(
    name: &str,
    track_id: TrackId,
    channel_filter: Option<u8>,
    tx: Sender<LiveMidiEvent>,
) -> Result<ActiveInputConn, String> {
    let input =
        MidiInput::new("resonance-input").map_err(|e| format!("create midi input: {e}"))?;
    let port = input
        .ports()
        .into_iter()
        .find(|p| input.port_name(p).map(|n| n == name).unwrap_or(false))
        .ok_or_else(|| format!("midi input port not found: {name}"))?;

    let filter = Arc::new(AtomicU8::new(encode_channel_filter(channel_filter)));
    let filter_callback = Arc::clone(&filter);
    let tx_callback = tx;
    let conn = input
        .connect(
            &port,
            "resonance-input-conn",
            move |_timestamp, raw, _| {
                // midir's `_timestamp` is platform-specific (microseconds
                // from connection-open on ALSA, but not portable), so
                // capture an `Instant` ourselves — it's monotonic and
                // we only ever subtract it from another `Instant`.
                let arrival = std::time::Instant::now();
                let filter_val = filter_callback.load(Ordering::Relaxed);
                if let Some(event) = parse_live_event(raw, track_id, filter_val, arrival) {
                    let _ = tx_callback.try_send(event);
                }
            },
            (),
        )
        .map_err(|e| format!("connect midi input {name}: {e}"))?;

    Ok(ActiveInputConn {
        device_name: name.to_string(),
        _conn: conn,
        channel_filter: filter,
    })
}

fn encode_channel_filter(c: Option<u8>) -> u8 {
    match c {
        Some(ch) if ch <= 15 => ch,
        _ => CHANNEL_OMNI,
    }
}

/// Parse a raw MIDI status byte slice from `midir` into a
/// [`LiveMidiEvent::InboundNoteOn`] / `InboundNoteOff`. Returns
/// `None` for non-note messages, channel-filtered messages, or
/// malformed data. A NoteOn with velocity 0 is normalised to
/// NoteOff to follow the running-status convention. `arrival` is the
/// wall-clock instant at which the midir callback fired and gets
/// stamped on the resulting event.
fn parse_live_event(
    raw: &[u8],
    track_id: TrackId,
    filter: u8,
    arrival: std::time::Instant,
) -> Option<LiveMidiEvent> {
    let status = *raw.first()?;
    let kind = status & 0xF0;
    let channel = status & 0x0F;
    if filter != CHANNEL_OMNI && channel != filter {
        return None;
    }
    match kind {
        0x90 if raw.len() >= 3 => {
            let note = raw[1] & 0x7F;
            let velocity = raw[2] & 0x7F;
            if velocity == 0 {
                Some(LiveMidiEvent::InboundNoteOff {
                    track_id,
                    note,
                    arrival,
                })
            } else {
                Some(LiveMidiEvent::InboundNoteOn {
                    track_id,
                    note,
                    velocity: velocity as f32 / 127.0,
                    arrival,
                })
            }
        }
        0x80 if raw.len() >= 3 => {
            let note = raw[1] & 0x7F;
            Some(LiveMidiEvent::InboundNoteOff {
                track_id,
                note,
                arrival,
            })
        }
        _ => None,
    }
}

// -----------------------------------------------------------------------------
// MIDI output
// -----------------------------------------------------------------------------

struct ActiveOutputConn {
    conn: MidiOutputConnection,
    refcount: usize,
    /// Every (channel, note) we've sent NoteOn for and not yet sent
    /// NoteOff for. The panic path uses this to send an explicit
    /// NoteOff per held note — far more reliable than CC 123, which
    /// some hardware synths and virtual MIDI bridges ignore.
    active_notes: HashSet<(u8, u8)>,
}

/// Per-device hardware MIDI output registry. Multiple tracks can
/// target the same physical device; the registry refcounts the
/// underlying [`MidiOutputConnection`] so the device opens once.
pub struct MidiOutputRegistry {
    connections: HashMap<String, ActiveOutputConn>,
    track_assignments: HashMap<TrackId, String>,
}

impl MidiOutputRegistry {
    pub fn new() -> Self {
        Self {
            connections: HashMap::new(),
            track_assignments: HashMap::new(),
        }
    }

    /// Assign or clear a track's MIDI output device. Refcounts the
    /// underlying device connection so multiple tracks can share one
    /// physical port.
    pub fn set_track_output(
        &mut self,
        track_id: TrackId,
        device_name: Option<String>,
    ) -> Result<(), String> {
        // Drop any previous assignment for this track first, sending
        // All Notes Off so a hardware synth doesn't sustain a stale
        // note across the reassign.
        if let Some(prev_name) = self.track_assignments.remove(&track_id) {
            self.send_all_notes_off(&prev_name);
            if let Some(active) = self.connections.get_mut(&prev_name) {
                active.refcount = active.refcount.saturating_sub(1);
                if active.refcount == 0 {
                    self.connections.remove(&prev_name);
                }
            }
        }

        let Some(name) = device_name else {
            return Ok(());
        };

        if let Some(active) = self.connections.get_mut(&name) {
            active.refcount += 1;
            self.track_assignments.insert(track_id, name);
            return Ok(());
        }

        let output = MidiOutput::new("resonance-output")
            .map_err(|e| format!("create midi output: {e}"))?;
        let port = output
            .ports()
            .into_iter()
            .find(|p| output.port_name(p).map(|n| n == name).unwrap_or(false))
            .ok_or_else(|| format!("midi output port not found: {name}"))?;
        let conn = output
            .connect(&port, "resonance-output-conn")
            .map_err(|e| format!("connect midi output {name}: {e}"))?;

        self.connections.insert(
            name.clone(),
            ActiveOutputConn {
                conn,
                refcount: 1,
                active_notes: HashSet::new(),
            },
        );
        self.track_assignments.insert(track_id, name);
        Ok(())
    }

    /// Drop any connection associated with a removed track.
    pub fn remove_track(&mut self, track_id: TrackId) {
        let _ = self.set_track_output(track_id, None);
    }

    /// Send a Bank Select (CC 0 MSB + CC 32 LSB) followed by a Program
    /// Change to the device assigned to `track_id` — the "patch send" an
    /// external-instrument track issues when its bank/program changes.
    /// `bank` / `program` of `None` skip that part of the message.
    ///
    /// Returns `true` when the patch reached a live connection, `false`
    /// when the track has no assigned device or its device is not
    /// currently connected (i.e. the MIDI output is offline). The caller
    /// uses the `false` result to report a recoverable device-offline
    /// event; the assignment is left intact so a replug reconnects.
    pub fn send_program_change(
        &mut self,
        track_id: TrackId,
        channel: u8,
        bank: Option<u16>,
        program: Option<u8>,
    ) -> bool {
        let Some(name) = self.track_assignments.get(&track_id).cloned() else {
            return false;
        };
        let Some(active) = self.connections.get_mut(&name) else {
            return false;
        };
        let ch = channel & 0x0F;
        if let Some(bank) = bank {
            let msb = ((bank >> 7) & 0x7F) as u8;
            let lsb = (bank & 0x7F) as u8;
            let _ = active.conn.send(&[0xB0 | ch, 0, msb]);
            let _ = active.conn.send(&[0xB0 | ch, 32, lsb]);
        }
        if let Some(program) = program {
            let _ = active.conn.send(&[0xC0 | ch, program & 0x7F]);
        }
        true
    }

    /// Test-only: returns the byte sequences that would be sent by
    /// `send_program_change` for a given bank/program/channel, verifying
    /// the CC0, CC32, Program Change ordering. This is a pure encoding
    /// function that matches what the realtime `send_program_change` emits.
    #[doc(hidden)]
    pub fn program_change_bytes(
        channel: u8,
        bank: Option<u16>,
        program: Option<u8>,
    ) -> Vec<Vec<u8>> {
        let mut result = Vec::new();
        let ch = channel & 0x0F;
        if let Some(bank) = bank {
            let msb = ((bank >> 7) & 0x7F) as u8;
            let lsb = (bank & 0x7F) as u8;
            result.push(vec![0xB0 | ch, 0, msb]);
            result.push(vec![0xB0 | ch, 32, lsb]);
        }
        if let Some(program) = program {
            result.push(vec![0xC0 | ch, program & 0x7F]);
        }
        result
    }

    /// Send a Note On to the device assigned to `track_id`, if any.
    pub fn send_note_on(&mut self, track_id: TrackId, channel: u8, note: u8, velocity: u8) {
        let Some(name) = self.track_assignments.get(&track_id).cloned() else {
            return;
        };
        if let Some(active) = self.connections.get_mut(&name) {
            let ch = channel & 0x0F;
            let n = note & 0x7F;
            let _ = active.conn.send(&[0x90 | ch, n, velocity & 0x7F]);
            active.active_notes.insert((ch, n));
        }
    }

    /// Send a Note Off to the device assigned to `track_id`, if any.
    pub fn send_note_off(&mut self, track_id: TrackId, channel: u8, note: u8) {
        let Some(name) = self.track_assignments.get(&track_id).cloned() else {
            return;
        };
        if let Some(active) = self.connections.get_mut(&name) {
            let ch = channel & 0x0F;
            let n = note & 0x7F;
            let _ = active.conn.send(&[0x80 | ch, n, 0]);
            active.active_notes.remove(&(ch, n));
        }
    }

    /// Full MIDI panic for one device: explicit Note Off for every note
    /// we know is held, then sustain pedal off (CC 64 = 0) and All Notes
    /// Off (CC 123 = 0) on every channel.
    ///
    /// The explicit Note Offs are the load-bearing part — CC 123 is
    /// ignored by some hardware synths and virtual MIDI bridges, and
    /// CC 64 wouldn't release a sustained note even when CC 123 fires.
    /// The CCs are belt-and-suspenders for any held note we missed.
    fn send_all_notes_off(&mut self, device_name: &str) {
        let Some(active) = self.connections.get_mut(device_name) else {
            return;
        };
        let held: Vec<(u8, u8)> = active.active_notes.drain().collect();
        for (ch, note) in held {
            let _ = active.conn.send(&[0x80 | ch, note, 0]);
        }
        for ch in 0u8..=15 {
            let _ = active.conn.send(&[0xB0 | ch, 64, 0]);
            let _ = active.conn.send(&[0xB0 | ch, 123, 0]);
        }
    }

    /// MIDI panic on every connected device. Called from the
    /// transport-stop and shutdown paths so stuck notes never outlive
    /// a `Stop` press or app quit.
    pub fn all_notes_off_everywhere(&mut self) {
        let names: Vec<String> = self.connections.keys().cloned().collect();
        for name in names {
            self.send_all_notes_off(&name);
        }
    }
}

impl Default for MidiOutputRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse raw MIDI bytes from a hardware input port. Exposed for
/// tests under `resonance-audio/tests/`. Stamps the result with a
/// fresh `Instant::now()` — tests that care about the value can
/// destructure the event and check it; the rest can ignore it.
pub fn parse_live_event_for_test(
    raw: &[u8],
    track_id: TrackId,
    channel_filter: Option<u8>,
) -> Option<LiveMidiEvent> {
    parse_live_event(
        raw,
        track_id,
        encode_channel_filter(channel_filter),
        std::time::Instant::now(),
    )
}
