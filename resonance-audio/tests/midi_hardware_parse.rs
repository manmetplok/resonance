//! Pure-data tests for the MIDI byte parser. No `midir` device
//! required — the parser is exposed via
//! `parse_live_event_for_test`.

use resonance_audio::midi_hardware::{parse_live_event_for_test, LiveMidiEvent};

#[test]
fn note_on_basic() {
    let event = parse_live_event_for_test(&[0x90, 60, 100], 7, None).unwrap();
    match event {
        LiveMidiEvent::InboundNoteOn {
            track_id,
            note,
            velocity,
            ..
        } => {
            assert_eq!(track_id, 7);
            assert_eq!(note, 60);
            assert!((velocity - 100.0 / 127.0).abs() < 1e-6);
        }
        _ => panic!("expected NoteOn"),
    }
}

#[test]
fn note_on_zero_velocity_is_note_off() {
    let event = parse_live_event_for_test(&[0x90, 60, 0], 7, None).unwrap();
    assert!(matches!(
        event,
        LiveMidiEvent::InboundNoteOff {
            track_id: 7,
            note: 60,
            ..
        }
    ));
}

#[test]
fn explicit_note_off() {
    let event = parse_live_event_for_test(&[0x80, 64, 50], 1, None).unwrap();
    assert!(matches!(
        event,
        LiveMidiEvent::InboundNoteOff {
            track_id: 1,
            note: 64,
            ..
        }
    ));
}

#[test]
fn channel_filter_blocks_other_channels() {
    // Status 0x91 = NoteOn on channel 1 (0-indexed).
    // Filter for channel 0 should drop it.
    let event = parse_live_event_for_test(&[0x91, 60, 100], 7, Some(0));
    assert!(event.is_none());
    // Filter for channel 1 admits it.
    let event = parse_live_event_for_test(&[0x91, 60, 100], 7, Some(1));
    assert!(matches!(event, Some(LiveMidiEvent::InboundNoteOn { .. })));
}

#[test]
fn omni_admits_any_channel() {
    for ch in 0..=15 {
        let status = 0x90 | ch;
        let event = parse_live_event_for_test(&[status, 40, 80], 1, None);
        assert!(
            matches!(event, Some(LiveMidiEvent::InboundNoteOn { .. })),
            "omni filter should admit channel {ch}"
        );
    }
}

#[test]
fn non_note_messages_return_none() {
    // CC.
    assert!(parse_live_event_for_test(&[0xB0, 7, 64], 1, None).is_none());
    // Pitch bend.
    assert!(parse_live_event_for_test(&[0xE0, 0, 64], 1, None).is_none());
    // Aftertouch.
    assert!(parse_live_event_for_test(&[0xD0, 64], 1, None).is_none());
}

#[test]
fn truncated_message_returns_none() {
    assert!(parse_live_event_for_test(&[0x90], 1, None).is_none());
    assert!(parse_live_event_for_test(&[0x90, 60], 1, None).is_none());
    assert!(parse_live_event_for_test(&[], 1, None).is_none());
}

#[test]
fn high_bit_in_data_byte_is_masked() {
    // Real-world devices only use 7-bit data, but defensively
    // strip any high bit set on the data bytes so a malformed
    // packet can't smuggle through a value > 127.
    let event = parse_live_event_for_test(&[0x90, 0xFF, 0xFF], 1, None).unwrap();
    match event {
        LiveMidiEvent::InboundNoteOn { note, velocity, .. } => {
            assert_eq!(note, 0x7F);
            assert!((velocity - 127.0 / 127.0).abs() < 1e-6);
        }
        _ => panic!("expected NoteOn"),
    }
}
