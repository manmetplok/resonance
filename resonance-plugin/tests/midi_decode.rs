//! Pure-function tests for the bridge's raw MIDI decoder (ba todo #1295).
//!
//! `tests/midi_events.rs` proves the decoder is *wired* — that a host's CC
//! actually reaches a plugin across the CLAP ABI. This file sweeps the value
//! ranges, which an ABI test cannot do exhaustively: every channel, every
//! 7-bit value, and the full 14-bit pitch-bend range.

use resonance_plugin::clap_bridge::decode_midi;
use resonance_plugin::ControlEvent;

/// Every controller message must decode on every one of the 16 channels —
/// a channel nibble leaking into the status match would silently drop 15/16
/// of a keyboard's output.
#[test]
fn every_channel_is_decoded() {
    for channel in 0..16u8 {
        assert_eq!(
            decode_midi([0xb0 | channel, 1, 127], 0),
            Some(ControlEvent::ControlChange {
                channel,
                controller: 1,
                value: 1.0,
                timing: 0,
            })
        );
        assert_eq!(
            decode_midi([0xd0 | channel, 127, 0], 0),
            Some(ControlEvent::ChannelPressure {
                channel,
                pressure: 1.0,
                timing: 0,
            })
        );
        assert_eq!(
            decode_midi([0xa0 | channel, 48, 0], 0),
            Some(ControlEvent::PolyPressure {
                channel,
                note: 48,
                pressure: 0.0,
                timing: 0,
            })
        );
        assert!(matches!(
            decode_midi([0xe0 | channel, 0, 64], 0),
            Some(ControlEvent::PitchBend { channel: c, .. }) if c == channel
        ));
    }
}

/// 7-bit controller values map onto the closed unit interval, monotonically.
#[test]
fn seven_bit_values_span_zero_to_one() {
    let mut previous = -1.0;
    for raw in 0..=127u8 {
        let Some(ControlEvent::ControlChange { value, .. }) = decode_midi([0xb0, 74, raw], 0) else {
            panic!("CC {raw} did not decode");
        };
        assert!(
            value > previous,
            "CC values must rise monotonically ({raw}: {value} after {previous})"
        );
        assert!((0.0..=1.0).contains(&value));
        previous = value;
    }
    assert_eq!(previous, 1.0, "127 must be exactly 1.0");
}

/// The 14-bit bend covers -1..=1 with an exact centre, monotonically. A
/// synth multiplies this by its bend range, so a non-zero centre detunes
/// every held note and a non-monotonic step is an audible zipper.
#[test]
fn pitch_bend_spans_minus_one_to_one_with_an_exact_centre() {
    let bend = |raw: u16| match decode_midi([0xe0, (raw & 0x7f) as u8, (raw >> 7) as u8], 0) {
        Some(ControlEvent::PitchBend { value, .. }) => value,
        other => panic!("bend {raw} decoded as {other:?}"),
    };

    assert_eq!(bend(0), -1.0);
    assert_eq!(bend(8192), 0.0, "centre must be exactly 0.0");
    assert_eq!(bend(16383), 1.0);

    let mut previous = f32::NEG_INFINITY;
    for raw in 0..=16383u16 {
        let value = bend(raw);
        assert!(value > previous, "bend must rise monotonically at {raw}");
        assert!((-1.0..=1.0).contains(&value));
        previous = value;
    }
}

/// Everything else is dropped. Note on/off in particular: the note port
/// prefers the CLAP dialect, and decoding notes from both would double-fire
/// against a host that sends them twice.
#[test]
fn unmapped_messages_are_dropped() {
    for status in [
        0x80, // note off
        0x90, // note on
        0xc0, // program change
        0xf0, // sysex
        0xf8, // clock
        0xfe, // active sensing
    ] {
        assert_eq!(
            decode_midi([status, 60, 100], 0),
            None,
            "status {status:#x} must not be decoded as a controller"
        );
    }
}

/// The block-relative timestamp is carried through untouched — it is what
/// makes controller changes sample-accurate inside the block.
#[test]
fn the_timing_is_preserved() {
    for timing in [0, 1, 63, 4095] {
        assert_eq!(
            decode_midi([0xb0, 11, 100], timing).map(|e| e.timing()),
            Some(timing)
        );
    }
}
