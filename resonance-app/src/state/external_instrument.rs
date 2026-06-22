//! GUI-side external-instrument track state (architecture doc #169, epic
//! #39).
//!
//! An "external instrument" track drives an outboard hardware/software
//! synth: it pairs the track's hardware **MIDI output** (device + channel)
//! with an **audio-return** input (device + ports) and adds the bits a plain
//! track has nowhere to put — a selected bank/program and a manual latency
//! offset. The MIDI-out device/channel, return device/ports and the
//! monitor / record-arm flags all live on [`crate::state::TrackState`]
//! (exactly one source of truth, mirroring the engine-side `Track`); this
//! struct only carries the external-specific config plus the runtime
//! device-offline flags the engine reports.
//!
//! The *presence* of an [`ExternalInstrumentState`] for a track is what
//! marks the track as being in external-instrument mode.

use std::collections::HashMap;

use resonance_audio::types::TrackId;
use resonance_common::ExternalInstrument;

use crate::state::TrackState;

/// Derived lifecycle status for an external-instrument track. Computed from
/// configured-ness (MIDI out + audio return), the monitor flag and the
/// device-offline flags — never stored, so it can never drift out of sync
/// with the underlying state. Drives the inspector badge + strip overlay
/// (the view lands in a later todo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalInstrumentStatus {
    /// No MIDI output picked yet — the track isn't wired to a synth.
    Unconfigured,
    /// A MIDI output is set but the route isn't fully live yet: the audio
    /// return is still missing or monitoring is off.
    Configuring,
    /// Fully paired (MIDI out + audio return) and monitoring — patch changes
    /// reach the synth and the return is audible.
    Live,
    /// A configured device (MIDI out or audio return) went offline. The
    /// route is preserved so a replug reconnects; the synth is unreachable
    /// until then.
    Offline,
}

/// Per-track external-instrument config + runtime device status.
///
/// `bank` / `program` / `latency_offset_samples` mirror the engine-side
/// [`ExternalInstrument`] config and are the user-editable, undoable fields.
/// The two `*_offline` flags are runtime device status reported by the
/// engine (`ExternalInstrumentMidiOutOffline` / `…ReturnInputOffline`) and
/// are *not* part of the undo snapshot — they reflect live hardware, not
/// project state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalInstrumentState {
    /// The track this config belongs to.
    pub track_id: TrackId,
    /// Selected MIDI bank as a combined 14-bit value (MSB << 7 | LSB), or
    /// `None` to leave the device on its current bank.
    pub bank: Option<u16>,
    /// Selected MIDI program (`0..=127`), or `None` to send no Program
    /// Change.
    pub program: Option<u8>,
    /// Manual latency offset in samples used to align the audio return with
    /// the timeline. Positive delays the return; negative pulls it earlier.
    pub latency_offset_samples: i64,
    /// True when the configured MIDI output device is offline (unplugged or
    /// a re-check found it gone). The route is preserved.
    pub midi_out_offline: bool,
    /// True when the configured audio-return input device is offline.
    pub return_input_offline: bool,
}

impl ExternalInstrumentState {
    /// A fresh external-instrument config for `track_id` with no bank /
    /// program selected, zero latency offset and both devices online.
    pub fn new(track_id: TrackId) -> Self {
        Self {
            track_id,
            bank: None,
            program: None,
            latency_offset_samples: 0,
            midi_out_offline: false,
            return_input_offline: false,
        }
    }

    /// Mirror an engine-side [`ExternalInstrument`] config, keeping the
    /// runtime offline flags untouched (the config echo doesn't report
    /// device status).
    pub fn apply_config(&mut self, config: &ExternalInstrument) {
        self.bank = config.bank;
        self.program = config.program;
        self.latency_offset_samples = config.latency_offset_samples;
    }

    /// The engine-side config form of this state (drops the runtime offline
    /// flags). Sent as `AudioCommand::SetExternalInstrument` and snapshotted
    /// for undo.
    pub fn config(&self) -> ExternalInstrument {
        ExternalInstrument {
            track_id: self.track_id,
            bank: self.bank,
            program: self.program,
            latency_offset_samples: self.latency_offset_samples,
        }
    }

    /// Derive the lifecycle [`ExternalInstrumentStatus`] from this config
    /// plus `track` (which owns the MIDI-out device, audio-return device and
    /// monitor flag). Offline wins over everything; then configured-ness and
    /// monitoring decide Live vs Configuring vs Unconfigured.
    pub fn status(&self, track: &TrackState) -> ExternalInstrumentStatus {
        if self.midi_out_offline || self.return_input_offline {
            return ExternalInstrumentStatus::Offline;
        }
        if track.midi_output_device.is_none() {
            return ExternalInstrumentStatus::Unconfigured;
        }
        let has_return = track.input_device_name.is_some();
        if has_return && track.monitor_enabled {
            ExternalInstrumentStatus::Live
        } else {
            ExternalInstrumentStatus::Configuring
        }
    }
}

/// Map of every external-instrument track to its config + device status.
/// Keyed by `TrackId`; absence means the track is a plain track. Lives on
/// [`crate::Resonance`] alongside the other runtime maps.
pub type ExternalInstrumentMap = HashMap<TrackId, ExternalInstrumentState>;
