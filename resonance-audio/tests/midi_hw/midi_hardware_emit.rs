//! Pure-data tests for the MIDI output encoders. No `midir` device
//! required — the encoders are exposed via `test_support`. They assert
//! the exact byte sequences for a 7-bit Control Change, a 7-bit NRPN, and
//! a 14-bit NRPN.

use resonance_audio::test_support::{encode_control_change, encode_nrpn};

#[test]
fn control_change_exact_bytes() {
    // CC 74 (filter cutoff) = 100 on channel 0.
    assert_eq!(encode_control_change(0, 74, 100), [0xB0, 74, 100]);
    // Channel goes in the low nibble of the status byte.
    assert_eq!(encode_control_change(5, 7, 64), [0xB5, 7, 64]);
}

#[test]
fn control_change_masks_out_of_range() {
    // Channel is masked to 0..=15, controller and value to 0..=127.
    assert_eq!(encode_control_change(0xFF, 0xFF, 0xFF), [0xBF, 0x7F, 0x7F]);
}

#[test]
fn nrpn_7bit_exact_bytes() {
    // 7-bit NRPN, parameter 1:36, value 64 on channel 0.
    // CC99=1, CC98=36, CC6=64. No CC38 for a 7-bit value.
    let bytes = encode_nrpn(0, 1, 36, 64, false);
    assert_eq!(
        bytes,
        vec![
            0xB0, 99, 1, // parameter MSB
            0xB0, 98, 36, // parameter LSB
            0xB0, 6, 64, // data-entry MSB carries the 7-bit value
        ]
    );
}

#[test]
fn nrpn_14bit_exact_bytes() {
    // 14-bit NRPN, parameter 1:36, value 8000 on channel 2.
    // 8000 = 0b1111101000000 -> MSB (>>7) = 62, LSB (&0x7F) = 64.
    let bytes = encode_nrpn(2, 1, 36, 8000, true);
    assert_eq!(8000u16 >> 7, 62);
    assert_eq!(8000u16 & 0x7F, 64);
    assert_eq!(
        bytes,
        vec![
            0xB2, 99, 1, // parameter MSB
            0xB2, 98, 36, // parameter LSB
            0xB2, 6, 62, // data-entry MSB = high 7 bits
            0xB2, 38, 64, // data-entry LSB = low 7 bits
        ]
    );
}

#[test]
fn nrpn_14bit_max_value() {
    // Full-scale 14-bit value 16383 -> MSB 127, LSB 127.
    let bytes = encode_nrpn(0, 0, 0, 16383, true);
    assert_eq!(&bytes[6..9], &[0xB0, 6, 127]);
    assert_eq!(&bytes[9..12], &[0xB0, 38, 127]);
}

#[test]
fn nrpn_buffer_length_matches_message_count() {
    // 7-bit NRPN = 3 CC messages, 14-bit = 4. Each CC is 3 bytes, so the
    // buffer chunks evenly into whole MIDI messages.
    assert_eq!(encode_nrpn(0, 0, 0, 0, false).len(), 9);
    assert_eq!(encode_nrpn(0, 0, 0, 0, true).len(), 12);
}
