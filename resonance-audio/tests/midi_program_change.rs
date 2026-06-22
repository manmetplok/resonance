//! Bank Select (CC0/CC32) + Program Change MIDI emission (todo #449, doc #169).
//!
//! Two things are exercised here, both against production code:
//!   * [`ExternalInstrument::patch_messages`] — the encoder that turns a
//!     track's selected bank/program into the exact byte sequence sent to the
//!     hardware: Bank Select MSB (CC 0) + LSB (CC 32) *first*, Program Change
//!     *last*, with 0-based values encoded as 7-bit MIDI data.
//!   * [`MidiOutputRegistry::send_program_change`] — the realtime emit path
//!     that hands those bytes to the device assigned to a track.
//!
//! The encoder and the emit path share the same encoding, so asserting on the
//! encoder pins the on-the-wire byte sequence ordering required by #449.

use resonance_audio::MidiOutputRegistry;
use resonance_audio::TrackId;
use resonance_common::ExternalInstrument;

/// Bank + program: CC0 (MSB) then CC32 (LSB) then Program Change, in that
/// order, on the requested channel. Bank 130 = MSB 1, LSB 2.
#[test]
fn patch_messages_bank_then_program_in_order() {
    let config = ExternalInstrument {
        track_id: 1,
        bank: Some(130),
        program: Some(42),
        latency_offset_samples: 0,
    };

    let msgs = config.patch_messages(3);

    assert_eq!(
        msgs,
        vec![
            vec![0xB3, 0, 1],  // Bank Select MSB on channel 3
            vec![0xB3, 32, 2], // Bank Select LSB on channel 3
            vec![0xC3, 42],    // Program Change on channel 3
        ],
        "bank select MSB+LSB must precede the program change"
    );
}

/// Program only: a single Program Change, no Bank Select.
#[test]
fn patch_messages_program_only() {
    let config = ExternalInstrument {
        track_id: 1,
        bank: None,
        program: Some(42),
        latency_offset_samples: 0,
    };

    assert_eq!(config.patch_messages(3), vec![vec![0xC3, 42]]);
}

/// Bank only: just the two Bank Select CCs, no Program Change.
#[test]
fn patch_messages_bank_only() {
    let config = ExternalInstrument {
        track_id: 1,
        bank: Some(130),
        program: None,
        latency_offset_samples: 0,
    };

    assert_eq!(
        config.patch_messages(3),
        vec![vec![0xB3, 0, 1], vec![0xB3, 32, 2]]
    );
}

/// Neither set: nothing to send.
#[test]
fn patch_messages_empty_when_unset() {
    let config = ExternalInstrument {
        track_id: 1,
        bank: None,
        program: None,
        latency_offset_samples: 0,
    };

    assert!(config.patch_messages(3).is_empty());
}

/// 7-bit data encoding at the boundaries: a 14-bit bank splits into two 7-bit
/// halves and the program is masked to 7 bits. Channel 0 keeps the status
/// nibbles clean. Bank 16383 = 0x3FFF = MSB 127, LSB 127.
#[test]
fn patch_messages_encodes_7bit_data() {
    let config = ExternalInstrument {
        track_id: 1,
        bank: Some(16383),
        program: Some(127),
        latency_offset_samples: 0,
    };

    assert_eq!(
        config.patch_messages(0),
        vec![
            vec![0xB0, 0, 127],
            vec![0xB0, 32, 127],
            vec![0xC0, 127],
        ]
    );
}

/// Channel is masked into the status nibble (0..=15); a channel of 15 lands in
/// the low nibble without corrupting the message-type nibble.
#[test]
fn patch_messages_channel_in_status_nibble() {
    let config = ExternalInstrument {
        track_id: 1,
        bank: Some(0),
        program: Some(0),
        latency_offset_samples: 0,
    };

    assert_eq!(
        config.patch_messages(15),
        vec![
            vec![0xBF, 0, 0],
            vec![0xBF, 32, 0],
            vec![0xCF, 0],
        ]
    );
}

/// The realtime emit path reports `false` when the track has no device
/// assigned — the caller turns that into a recoverable "MIDI out offline"
/// event rather than silently dropping the patch.
#[test]
fn send_program_change_false_without_device() {
    let mut registry = MidiOutputRegistry::new();
    let track_id: TrackId = 42;

    assert!(
        !registry.send_program_change(track_id, 0, Some(0), Some(0)),
        "send_program_change must report failure when no device is assigned"
    );
}
