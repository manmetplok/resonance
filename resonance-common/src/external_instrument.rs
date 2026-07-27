//! External-instrument track config, shared across the engine, app state and
//! project I/O (architecture doc #169, epic #39).
//!
//! An "external instrument" track drives an outboard hardware/software synth:
//! it pairs a hardware **MIDI output** (the track's existing
//! `midi_output_device` / `midi_output_channel`) with an **audio return**
//! input (the track's existing `input_device_name` / input port), and adds the
//! bits that have no home on a plain track — the selected bank/program and a
//! manual latency offset that aligns the round-tripped audio with the timeline.
//!
//! The struct lives here so the realtime engine (`resonance-audio`), the app
//! (`resonance-app`) and project persistence all agree on the shape of the
//! config. Device/channel and monitor/record-arm are *not* duplicated here —
//! they stay on the engine-side `Track` so there is exactly one source of
//! truth for them.

use serde::{Deserialize, Serialize};

use crate::automation::TrackId;

/// The per-track extra config that turns a track into an external instrument.
///
/// The MIDI output device + channel and the audio-return device + channels are
/// read from the track itself; this struct only carries what a plain track has
/// nowhere to put. The *presence* of an `ExternalInstrument` for a track is
/// what marks the track as being in external-instrument mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExternalInstrument {
    /// The track this config belongs to.
    pub track_id: TrackId,
    /// Selected MIDI bank as a combined 14-bit value (MSB << 7 | LSB), or
    /// `None` to leave the device on its current bank. Sent as Bank Select
    /// CC 0 (MSB) + CC 32 (LSB) ahead of the Program Change.
    pub bank: Option<u16>,
    /// Selected MIDI program (`0..=127`), or `None` to send no Program Change.
    pub program: Option<u8>,
    /// Manual latency offset in samples used to align the audio return with
    /// the timeline. Positive delays the return; negative pulls it earlier.
    /// Applying it to the audio path is a later step — this is the config only.
    pub latency_offset_samples: i64,
}

impl ExternalInstrument {
    /// A fresh external-instrument config for `track_id` with no bank/program
    /// selected and zero latency offset.
    pub fn new(track_id: TrackId) -> Self {
        Self {
            track_id,
            bank: None,
            program: None,
            latency_offset_samples: 0,
        }
    }

    /// The MIDI messages that select this config's bank + program on `channel`
    /// (0-indexed, `0..=15`). Bank Select MSB/LSB come first (only when a bank
    /// is set), then the Program Change (only when a program is set). Returns
    /// an empty `Vec` when neither is set — there is nothing to send.
    ///
    /// Each entry is a complete MIDI message ready to hand to the output port.
    pub fn patch_messages(&self, channel: u8) -> Vec<Vec<u8>> {
        let ch = channel & 0x0F;
        let mut msgs = Vec::new();
        if let Some(bank) = self.bank {
            let msb = ((bank >> 7) & 0x7F) as u8;
            let lsb = (bank & 0x7F) as u8;
            msgs.push(vec![0xB0 | ch, 0, msb]);
            msgs.push(vec![0xB0 | ch, 32, lsb]);
        }
        if let Some(program) = self.program {
            msgs.push(vec![0xC0 | ch, program & 0x7F]);
        }
        msgs
    }
}
