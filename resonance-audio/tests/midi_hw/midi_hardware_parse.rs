//! Pure-data tests for the MIDI byte parser. No `midir` device
//! required — the parser is exposed via
//! `parse_live_event_for_test`.

use resonance_audio::test_support::{parse_live_event_for_test, LiveMidiEvent};

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
fn controllers_parse_as_raw_midi_for_the_instrument() {
    // Code review HOST-13: CC, pitch bend and aftertouch reach the
    // instrument, as their raw bytes (channel kept, data masked).
    let raw = |bytes: &[u8]| match parse_live_event_for_test(bytes, 1, None) {
        Some(LiveMidiEvent::InboundMidi { track_id, data, .. }) => {
            assert_eq!(track_id, 1);
            data
        }
        other => panic!("{bytes:02x?} parsed as {other:?}"),
    };
    assert_eq!(raw(&[0xB3, 1, 0xC0]), [0xB3, 1, 0x40], "mod wheel, channel 4");
    assert_eq!(raw(&[0xE0, 0, 64]), [0xE0, 0, 64], "pitch bend");
    assert_eq!(raw(&[0xD0, 64]), [0xD0, 64, 0], "channel aftertouch");
    assert_eq!(raw(&[0xA0, 60, 90]), [0xA0, 60, 90], "poly aftertouch");
    // Truncated controllers, program change and system messages are not.
    assert!(parse_live_event_for_test(&[0xB0, 7], 1, None).is_none());
    assert!(parse_live_event_for_test(&[0xC0, 5], 1, None).is_none());
    assert!(parse_live_event_for_test(&[0xF8], 1, None).is_none());
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
