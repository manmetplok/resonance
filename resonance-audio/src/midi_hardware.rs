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
use thiserror::Error;

use crate::types::{EngineError, EngineErrorKind, TrackId};

/// Failure opening or connecting a hardware MIDI input/output/control-
/// surface port. `surface` names which registry raised it (`"midi"` or
/// `"control-surface"`), matching the historical message text (`"create
/// midi input: ..."`, `"control-surface input port not found: ..."`, ...).
#[derive(Debug, Error)]
pub enum MidiHardwareError {
    #[error("create {surface} input: {source}")]
    CreateInput {
        surface: &'static str,
        #[source]
        source: midir::InitError,
    },
    #[error("create {surface} output: {source}")]
    CreateOutput {
        surface: &'static str,
        #[source]
        source: midir::InitError,
    },
    #[error("{surface} input port not found: {name}")]
    InputPortNotFound { surface: &'static str, name: String },
    #[error("{surface} output port not found: {name}")]
    OutputPortNotFound { surface: &'static str, name: String },
    #[error("connect {surface} input {name}: {source}")]
    ConnectInput {
        surface: &'static str,
        name: String,
        #[source]
        source: midir::ConnectError<MidiInput>,
    },
    #[error("connect {surface} output {name}: {source}")]
    ConnectOutput {
        surface: &'static str,
        name: String,
        #[source]
        source: midir::ConnectError<MidiOutput>,
    },
}

impl From<MidiHardwareError> for EngineError {
    fn from(e: MidiHardwareError) -> Self {
        let kind = match &e {
            MidiHardwareError::InputPortNotFound { .. }
            | MidiHardwareError::OutputPortNotFound { .. } => EngineErrorKind::NotFound,
            MidiHardwareError::CreateInput { .. }
            | MidiHardwareError::CreateOutput { .. }
            | MidiHardwareError::ConnectInput { .. }
            | MidiHardwareError::ConnectOutput { .. } => EngineErrorKind::Io,
        };
        EngineError::new(kind, e.to_string())
    }
}

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
    /// A MIDI 1.0 channel message other than a note — control change,
    /// pitch bend, channel or poly aftertouch — on a track's input port,
    /// as its raw bytes (status with its channel, then the data bytes; a
    /// two-byte message is padded with 0). The audio thread forwards it
    /// to the track's instrument as `CLAP_EVENT_MIDI` when the
    /// instrument's note port takes MIDI (code review HOST-13); it is not
    /// recorded or sent Thru yet.
    InboundMidi {
        track_id: TrackId,
        data: [u8; 3],
        arrival: std::time::Instant,
    },
}

/// Control-surface MIDI input drained from the midir-spawned thread on
/// the engine control thread. Unlike [`LiveMidiEvent`] this is not tied
/// to a track: it carries the raw channel so the mapping layer can match
/// a binding's [`ControlSource`] by `(channel, cc)` / `(channel, note)`.
///
/// Binding application + soft-takeover consume these (doc #167 §2 E3,
/// todo #430); for now the engine drains and drops them.
///
/// [`ControlSource`]: resonance-common's `midi_map::ControlSource`
#[derive(Debug, Clone)]
#[allow(dead_code)] // fields consumed by binding application in todo #430 (E3)
pub enum LiveControlEvent {
    /// Control Change (`0xB0`): a knob/fader/encoder move.
    Cc {
        channel: u8,
        cc: u8,
        value: u8,
        arrival: std::time::Instant,
    },
    /// Note On/Off (`0x90`/`0x80`): a pad/button used as a toggle or
    /// trigger. `velocity == 0` means the key was released (either an
    /// explicit Note Off or a Note On with velocity 0, per the
    /// running-status convention), so the mapping layer can treat
    /// `velocity > 0` as "pressed".
    Note {
        channel: u8,
        note: u8,
        velocity: u8,
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
    ) -> Result<(), MidiHardwareError> {
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
) -> Result<ActiveInputConn, MidiHardwareError> {
    let input = MidiInput::new("resonance-input").map_err(|e| MidiHardwareError::CreateInput {
        surface: "midi",
        source: e,
    })?;
    let port = input
        .ports()
        .into_iter()
        .find(|p| input.port_name(p).map(|n| n == name).unwrap_or(false))
        .ok_or_else(|| MidiHardwareError::InputPortNotFound {
            surface: "midi",
            name: name.to_string(),
        })?;

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
        .map_err(|e| MidiHardwareError::ConnectInput {
            surface: "midi",
            name: name.to_string(),
            source: e,
        })?;

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
/// [`LiveMidiEvent::InboundNoteOn`] / `InboundNoteOff`, or an
/// [`LiveMidiEvent::InboundMidi`] for a control change, pitch bend or
/// aftertouch. Returns `None` for other messages (program change,
/// system), channel-filtered messages, or malformed data. A NoteOn with velocity 0 is normalised to
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
        // Poly aftertouch, control change, pitch bend: three bytes.
        0xA0 | 0xB0 | 0xE0 if raw.len() >= 3 => Some(LiveMidiEvent::InboundMidi {
            track_id,
            data: [status, raw[1] & 0x7F, raw[2] & 0x7F],
            arrival,
        }),
        // Channel aftertouch: two.
        0xD0 if raw.len() >= 2 => Some(LiveMidiEvent::InboundMidi {
            track_id,
            data: [status, raw[1] & 0x7F, 0],
            arrival,
        }),
        _ => None,
    }
}

// -----------------------------------------------------------------------------
// Control surface input
// -----------------------------------------------------------------------------

struct ActiveControlConn {
    device_name: String,
    _conn: MidiInputConnection<()>,
}

/// A single, track-independent MIDI input dedicated to a hardware
/// control surface (knobs, faders, pads, transport). Mirrors the
/// per-track [`MidiInputRegistry`] threading model — midir calls stay on
/// the engine control thread, the spawned midir thread parses bytes and
/// pushes [`LiveControlEvent`]s into a bounded crossbeam channel — but
/// holds at most one connection and listens omni (the binding layer
/// matches on the per-event channel rather than a port-wide filter).
pub struct ControlSurfaceInput {
    conn: Option<ActiveControlConn>,
    /// Set when the chosen device isn't currently present; reconciled on
    /// the next [`Self::reconcile`] so re-plugging recovers without the
    /// user re-picking it.
    pending: Option<String>,
    tx: Sender<LiveControlEvent>,
}

impl ControlSurfaceInput {
    pub fn new(tx: Sender<LiveControlEvent>) -> Self {
        Self {
            conn: None,
            pending: None,
            tx,
        }
    }

    /// Pick the control-surface input device. `device_name = None`
    /// closes any open port and clears a pending request. A device that
    /// isn't currently present is stored as pending (no error) and opened
    /// by [`Self::reconcile`] once it appears.
    ///
    /// Wired to `AudioCommand::SetControlSurfaceInput` in todo #429 (E2).
    #[allow(dead_code)]
    pub fn set_input(&mut self, device_name: Option<String>) -> Result<(), MidiHardwareError> {
        // Already connected to the requested device — nothing to do.
        if let Some(active) = &self.conn {
            if Some(&active.device_name) == device_name.as_ref() {
                return Ok(());
            }
        }

        // Drop any previous connection (closes the port) and pending want.
        self.conn = None;
        self.pending = None;

        let Some(name) = device_name else {
            return Ok(());
        };

        match open_control_input(&name, self.tx.clone()) {
            Ok(active) => {
                self.conn = Some(active);
                Ok(())
            }
            Err(_) => {
                self.pending = Some(name);
                Ok(())
            }
        }
    }

    /// Try to open a pending control-surface device that has just
    /// appeared. Called after every input enumeration so a freshly
    /// plugged-in surface starts working without user intervention.
    pub fn reconcile(&mut self) {
        let Some(name) = self.pending.clone() else {
            return;
        };
        if let Ok(active) = open_control_input(&name, self.tx.clone()) {
            self.conn = Some(active);
            self.pending = None;
        }
    }
}

/// Open the named MIDI input as the control surface and wire up the
/// message callback. The callback stamps a monotonic `arrival`, parses
/// the bytes into a [`LiveControlEvent`], and pushes it onto the bounded
/// channel; a full channel drops the event rather than blocking the
/// midir thread.
fn open_control_input(
    name: &str,
    tx: Sender<LiveControlEvent>,
) -> Result<ActiveControlConn, MidiHardwareError> {
    let input = MidiInput::new("resonance-control-surface").map_err(|e| {
        MidiHardwareError::CreateInput {
            surface: "control-surface",
            source: e,
        }
    })?;
    let port = input
        .ports()
        .into_iter()
        .find(|p| input.port_name(p).map(|n| n == name).unwrap_or(false))
        .ok_or_else(|| MidiHardwareError::InputPortNotFound {
            surface: "control-surface",
            name: name.to_string(),
        })?;

    let tx_callback = tx;
    let conn = input
        .connect(
            &port,
            "resonance-control-surface-conn",
            move |_timestamp, raw, _| {
                // midir's `_timestamp` is platform-specific; capture a
                // monotonic `Instant` ourselves (see `open_input`).
                let arrival = std::time::Instant::now();
                if let Some(event) = parse_control_event(raw, arrival) {
                    let _ = tx_callback.try_send(event);
                }
            },
            (),
        )
        .map_err(|e| MidiHardwareError::ConnectInput {
            surface: "control-surface",
            name: name.to_string(),
            source: e,
        })?;

    Ok(ActiveControlConn {
        device_name: name.to_string(),
        _conn: conn,
    })
}

/// Parse a raw MIDI status byte slice from the control surface into a
/// [`LiveControlEvent`]. Emits `Cc` for `0xB0`, `Note` for `0x90`/`0x80`
/// (a Note On with velocity 0 collapses to `velocity = 0`, matching the
/// running-status note-off convention). Returns `None` for any other
/// message kind or malformed/truncated data. Listens omni — the channel
/// is captured on the event for the binding layer to match.
fn parse_control_event(raw: &[u8], arrival: std::time::Instant) -> Option<LiveControlEvent> {
    let status = *raw.first()?;
    let kind = status & 0xF0;
    let channel = status & 0x0F;
    match kind {
        0xB0 if raw.len() >= 3 => Some(LiveControlEvent::Cc {
            channel,
            cc: raw[1] & 0x7F,
            value: raw[2] & 0x7F,
            arrival,
        }),
        0x90 if raw.len() >= 3 => Some(LiveControlEvent::Note {
            channel,
            note: raw[1] & 0x7F,
            // velocity 0 stays 0 → the mapping layer reads it as release.
            velocity: raw[2] & 0x7F,
            arrival,
        }),
        0x80 if raw.len() >= 3 => Some(LiveControlEvent::Note {
            channel,
            note: raw[1] & 0x7F,
            velocity: 0,
            arrival,
        }),
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
    ) -> Result<(), MidiHardwareError> {
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

        let output = MidiOutput::new("resonance-output").map_err(|e| {
            MidiHardwareError::CreateOutput {
                surface: "midi",
                source: e,
            }
        })?;
        let port = output
            .ports()
            .into_iter()
            .find(|p| output.port_name(p).map(|n| n == name).unwrap_or(false))
            .ok_or_else(|| MidiHardwareError::OutputPortNotFound {
                surface: "midi",
                name: name.clone(),
            })?;
        let conn = output
            .connect(&port, "resonance-output-conn")
            .map_err(|e| MidiHardwareError::ConnectOutput {
                surface: "midi",
                name: name.clone(),
                source: e,
            })?;

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

    /// Send a raw Control Change to the device assigned to `track_id`,
    /// if any. Reuses the same per-device sender lookup as
    /// [`Self::send_note_on`].
    // Emission primitive only (ba todo #718); the timeline/automation
    // call sites land in a follow-up todo.
    #[allow(dead_code)]
    pub fn send_control_change(&mut self, track_id: TrackId, channel: u8, cc: u8, value: u8) {
        let Some(name) = self.track_assignments.get(&track_id).cloned() else {
            return;
        };
        if let Some(active) = self.connections.get_mut(&name) {
            let _ = active.conn.send(&encode_control_change(channel, cc, value));
        }
    }

    /// Send an NRPN (Non-Registered Parameter Number) to the device
    /// assigned to `track_id`, if any. The NRPN is encoded by
    /// [`encode_nrpn`] as a run of Control Change messages; each 3-byte
    /// CC message is sent individually since `midir` expects one MIDI
    /// message per `send` call.
    #[allow(dead_code)]
    pub fn send_nrpn(
        &mut self,
        track_id: TrackId,
        channel: u8,
        msb: u8,
        lsb: u8,
        value: u16,
        fourteen_bit: bool,
    ) {
        let Some(name) = self.track_assignments.get(&track_id).cloned() else {
            return;
        };
        if let Some(active) = self.connections.get_mut(&name) {
            for msg in encode_nrpn(channel, msb, lsb, value, fourteen_bit).chunks_exact(3) {
                let _ = active.conn.send(msg);
            }
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

/// Control Change status nibble (channel goes in the low nibble).
const CC_STATUS: u8 = 0xB0;
/// NRPN parameter-select controllers: MSB then LSB.
const CC_NRPN_PARAM_MSB: u8 = 99;
const CC_NRPN_PARAM_LSB: u8 = 98;
/// Data-entry controllers used to carry the NRPN value.
const CC_DATA_ENTRY_MSB: u8 = 6;
const CC_DATA_ENTRY_LSB: u8 = 38;

/// Encode a single Control Change message `[status|channel, cc, value]`.
/// Channel is masked to 0..=15 and the controller/value to 0..=127 so an
/// out-of-range argument can neither corrupt the status byte nor smuggle a
/// high bit into a data byte.
pub fn encode_control_change(channel: u8, cc: u8, value: u8) -> [u8; 3] {
    [CC_STATUS | (channel & 0x0F), cc & 0x7F, value & 0x7F]
}

/// Encode an NRPN as the standard sequence of Control Change messages:
/// CC99 = parameter MSB, CC98 = parameter LSB, CC6 = data-entry MSB, and
/// — only when `fourteen_bit` — CC38 = data-entry LSB.
///
/// For a 7-bit parameter the 7-bit `value` (0..=127) is carried in the
/// single CC6 data-entry MSB. For a 14-bit parameter `value` (0..=16383)
/// is split into its high 7 bits (CC6) and low 7 bits (CC38). The returned
/// buffer is the flat byte stream, 3 bytes per CC message (9 bytes for a
/// 7-bit NRPN, 12 for a 14-bit one).
pub fn encode_nrpn(channel: u8, msb: u8, lsb: u8, value: u16, fourteen_bit: bool) -> Vec<u8> {
    let ch = channel & 0x0F;
    let mut bytes = Vec::with_capacity(if fourteen_bit { 12 } else { 9 });
    bytes.extend_from_slice(&encode_control_change(ch, CC_NRPN_PARAM_MSB, msb));
    bytes.extend_from_slice(&encode_control_change(ch, CC_NRPN_PARAM_LSB, lsb));
    if fourteen_bit {
        bytes.extend_from_slice(&encode_control_change(ch, CC_DATA_ENTRY_MSB, (value >> 7) as u8));
        bytes.extend_from_slice(&encode_control_change(ch, CC_DATA_ENTRY_LSB, value as u8));
    } else {
        bytes.extend_from_slice(&encode_control_change(ch, CC_DATA_ENTRY_MSB, value as u8));
    }
    bytes
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

/// Parse raw control-surface MIDI bytes into a [`LiveControlEvent`].
/// Exposed for tests under `resonance-audio/tests/`. Stamps the result
/// with a fresh `Instant::now()`; tests that don't care can ignore it.
pub fn parse_control_event_for_test(raw: &[u8]) -> Option<LiveControlEvent> {
    parse_control_event(raw, std::time::Instant::now())
}
