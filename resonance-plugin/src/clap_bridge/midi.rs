//! Decoding raw MIDI 1.0 channel-voice messages into [`ControlEvent`]s
//! (ba todo #1295).
//!
//! The CLAP note dialect expresses notes and per-note expression, but it has
//! no concept of a control change: CLAP's position is that a host either maps
//! CCs onto parameters or passes them through as raw MIDI. So the bridge
//! declares the MIDI dialect alongside the CLAP one on its note port
//! (`clap_bridge/ports.rs`) and decodes the channel-voice messages that carry
//! the three controller kinds a synth actually modulates from: control
//! change, aftertouch (channel and polyphonic) and pitch bend.
//!
//! Everything here is a pure function on the 3 raw bytes — no state, no
//! allocation — so it is safe on the audio thread and testable on its own.

use crate::plugin::ControlEvent;

/// MIDI status nibbles (the high nibble of the status byte).
const POLY_PRESSURE: u8 = 0xA0;
const CONTROL_CHANGE: u8 = 0xB0;
const CHANNEL_PRESSURE: u8 = 0xD0;
const PITCH_BEND: u8 = 0xE0;

/// Normalise a 7-bit MIDI value to `0.0..=1.0`.
#[inline]
fn norm7(value: u8) -> f32 {
    (value & 0x7f) as f32 / 127.0
}

/// Decode one raw MIDI 1.0 message into the plugin-facing controller event,
/// or `None` for a message this bridge does not forward (note on/off — those
/// arrive through the CLAP dialect — program change, system messages, …).
///
/// `data` is the CLAP `clap_event_midi` payload: status byte, then up to two
/// data bytes.
#[inline]
pub fn decode_midi(data: [u8; 3], timing: u32) -> Option<ControlEvent> {
    let status = data[0] & 0xf0;
    let channel = data[0] & 0x0f;
    match status {
        CONTROL_CHANGE => Some(ControlEvent::ControlChange {
            channel,
            controller: data[1] & 0x7f,
            value: norm7(data[2]),
            timing,
        }),
        CHANNEL_PRESSURE => Some(ControlEvent::ChannelPressure {
            channel,
            // Channel pressure is a single-data-byte message.
            pressure: norm7(data[1]),
            timing,
        }),
        POLY_PRESSURE => Some(ControlEvent::PolyPressure {
            channel,
            note: data[1] & 0x7f,
            pressure: norm7(data[2]),
            timing,
        }),
        PITCH_BEND => Some(ControlEvent::PitchBend {
            channel,
            value: pitch_bend_value(data[1], data[2]),
            timing,
        }),
        _ => None,
    }
}

/// Turn the 14-bit little-endian pitch-bend pair into `-1.0..=1.0`.
///
/// Centre (8192) is exactly 0.0. The two halves are deliberately scaled by
/// different divisors — 8192 below centre, 8191 above — so that both extremes
/// reach exactly ±1.0; scaling both by 8192 would cap a full upward bend at
/// 0.99988 and leave a synth's bend range fractionally short at the top.
#[inline]
fn pitch_bend_value(lsb: u8, msb: u8) -> f32 {
    let raw = ((msb & 0x7f) as i32) << 7 | (lsb & 0x7f) as i32;
    let offset = raw - 8192;
    if offset < 0 {
        offset as f32 / 8192.0
    } else {
        offset as f32 / 8191.0
    }
}
