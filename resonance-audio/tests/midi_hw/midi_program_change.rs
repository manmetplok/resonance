//! Bank Select (CC0/CC32) + Program Change MIDI emission (todo #449, doc #169).
//!
//! Two independent implementations of the Bank Select + Program Change
//! encoding are exercised here:
//!   * [`ExternalInstrument::patch_messages`] in resonance-common — heap-allocated Vecs,
//!     used for patch selection in the app.
//!   * [`MidiOutputRegistry::send_program_change`] in resonance-audio — realtime emit
//!     path that sends bytes directly to hardware MIDI devices via midir, with
//!     stack-based arrays to avoid audio-thread allocation.
//!
//! Both implementations must emit the same byte sequences (CC0 MSB, CC32 LSB,
//! then Program Change) with correct 7-bit encoding. The tests verify they match.

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

/// Verify that the emit-path encoding (MidiOutputRegistry::program_change_bytes)
/// matches the app-side encoding (ExternalInstrument::patch_messages) for
/// the same bank/program/channel inputs. This ensures both independent
/// implementations produce identical on-the-wire byte sequences.
#[test]
fn emit_path_encoding_matches_patch_messages() {
    // Bank 130 on channel 3: MSB=1, LSB=2
    let channel = 3u8;
    let bank = Some(130u16);
    let program = Some(42u8);

    let emit_bytes = MidiOutputRegistry::program_change_bytes(channel, bank, program);
    let config = ExternalInstrument {
        track_id: 1,
        bank,
        program,
        latency_offset_samples: 0,
    };
    let patch_bytes = config.patch_messages(channel);

    assert_eq!(
        emit_bytes, patch_bytes,
        "emit-path encoding must match patch_messages encoding for bank+program on channel {}",
        channel
    );
}

/// Verify emit-path encoding for program-only case.
#[test]
fn emit_path_encoding_program_only() {
    let channel = 5u8;
    let bank = None;
    let program = Some(64u8);

    let emit_bytes = MidiOutputRegistry::program_change_bytes(channel, bank, program);
    let config = ExternalInstrument {
        track_id: 1,
        bank,
        program,
        latency_offset_samples: 0,
    };
    let patch_bytes = config.patch_messages(channel);

    assert_eq!(emit_bytes, patch_bytes);
}

/// Verify emit-path encoding for bank-only case.
#[test]
fn emit_path_encoding_bank_only() {
    let channel = 0u8;
    let bank = Some(256u16); // MSB=2, LSB=0
    let program = None;

    let emit_bytes = MidiOutputRegistry::program_change_bytes(channel, bank, program);
    let config = ExternalInstrument {
        track_id: 1,
        bank,
        program,
        latency_offset_samples: 0,
    };
    let patch_bytes = config.patch_messages(channel);

    assert_eq!(emit_bytes, patch_bytes);
}

/// Verify emit-path encoding for neither bank nor program.
#[test]
fn emit_path_encoding_empty() {
    let channel = 10u8;
    let bank = None;
    let program = None;

    let emit_bytes = MidiOutputRegistry::program_change_bytes(channel, bank, program);
    let config = ExternalInstrument {
        track_id: 1,
        bank,
        program,
        latency_offset_samples: 0,
    };
    let patch_bytes = config.patch_messages(channel);

    assert_eq!(emit_bytes, patch_bytes);
    assert!(emit_bytes.is_empty());
}

/// Verify emit-path encoding at 7-bit boundaries (max values).
#[test]
fn emit_path_encoding_7bit_boundaries() {
    let channel = 0u8;
    let bank = Some(16383u16); // 0x3FFF = MSB 127, LSB 127
    let program = Some(127u8);

    let emit_bytes = MidiOutputRegistry::program_change_bytes(channel, bank, program);
    let config = ExternalInstrument {
        track_id: 1,
        bank,
        program,
        latency_offset_samples: 0,
    };
    let patch_bytes = config.patch_messages(channel);

    assert_eq!(
        emit_bytes, patch_bytes,
        "7-bit boundary encoding: bank 16383 = MSB 127, LSB 127; program 127"
    );
    assert_eq!(
        emit_bytes,
        vec![
            vec![0xB0, 0, 127],
            vec![0xB0, 32, 127],
            vec![0xC0, 127],
        ]
    );
}
