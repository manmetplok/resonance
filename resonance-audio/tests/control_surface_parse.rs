//! Pure-data tests for the control-surface MIDI parser. No `midir`
//! device required — the parser is exposed via
//! `parse_control_event_for_test`. The final test mirrors the midir
//! callback (parse → push onto a bounded channel) to prove events reach
//! the drain channel the engine control thread reads.

use resonance_audio::__test_support::{parse_control_event_for_test, LiveControlEvent};

#[test]
fn cc_basic() {
    // Status 0xB2 = CC on channel 2 (0-indexed), CC #7 (volume), value 100.
    let event = parse_control_event_for_test(&[0xB2, 7, 100]).unwrap();
    match event {
        LiveControlEvent::Cc {
            channel,
            cc,
            value,
            ..
        } => {
            assert_eq!(channel, 2);
            assert_eq!(cc, 7);
            assert_eq!(value, 100);
        }
        _ => panic!("expected Cc"),
    }
}

#[test]
fn note_on_is_note_with_velocity() {
    let event = parse_control_event_for_test(&[0x90, 60, 100]).unwrap();
    match event {
        LiveControlEvent::Note {
            channel,
            note,
            velocity,
            ..
        } => {
            assert_eq!(channel, 0);
            assert_eq!(note, 60);
            assert_eq!(velocity, 100);
        }
        _ => panic!("expected Note"),
    }
}

#[test]
fn note_on_zero_velocity_collapses_to_release() {
    // Running-status note-off: NoteOn with velocity 0 → velocity 0.
    let event = parse_control_event_for_test(&[0x90, 60, 0]).unwrap();
    assert!(matches!(
        event,
        LiveControlEvent::Note {
            note: 60,
            velocity: 0,
            ..
        }
    ));
}

#[test]
fn explicit_note_off_has_zero_velocity() {
    // Note Off carries a release velocity (50) but the mapping layer
    // only cares about pressed-vs-released, so it normalises to 0.
    let event = parse_control_event_for_test(&[0x81, 64, 50]).unwrap();
    match event {
        LiveControlEvent::Note {
            channel,
            note,
            velocity,
            ..
        } => {
            assert_eq!(channel, 1);
            assert_eq!(note, 64);
            assert_eq!(velocity, 0);
        }
        _ => panic!("expected Note"),
    }
}

#[test]
fn channel_is_captured_for_all_channels() {
    // The surface input listens omni; the parser captures the channel
    // on every CC so the binding layer can match per-channel sources.
    for ch in 0..=15 {
        let status = 0xB0 | ch;
        let event = parse_control_event_for_test(&[status, 10, 64]).unwrap();
        match event {
            LiveControlEvent::Cc { channel, .. } => assert_eq!(channel, ch),
            _ => panic!("expected Cc on channel {ch}"),
        }
    }
}

#[test]
fn unsupported_messages_return_none() {
    // Pitch bend.
    assert!(parse_control_event_for_test(&[0xE0, 0, 64]).is_none());
    // Channel aftertouch.
    assert!(parse_control_event_for_test(&[0xD0, 64]).is_none());
    // Program change.
    assert!(parse_control_event_for_test(&[0xC0, 5]).is_none());
}

#[test]
fn truncated_message_returns_none() {
    assert!(parse_control_event_for_test(&[0xB0]).is_none());
    assert!(parse_control_event_for_test(&[0xB0, 7]).is_none());
    assert!(parse_control_event_for_test(&[0x90, 60]).is_none());
    assert!(parse_control_event_for_test(&[0x80, 64]).is_none());
    assert!(parse_control_event_for_test(&[]).is_none());
}

#[test]
fn high_bit_in_data_byte_is_masked() {
    // Defensively strip the high bit so a malformed packet can't smuggle
    // a value > 127 through to the binding range math.
    let event = parse_control_event_for_test(&[0xB0, 0xFF, 0xFF]).unwrap();
    match event {
        LiveControlEvent::Cc { cc, value, .. } => {
            assert_eq!(cc, 0x7F);
            assert_eq!(value, 0x7F);
        }
        _ => panic!("expected Cc"),
    }
}

#[test]
fn parsed_events_reach_the_drain_channel() {
    // Mirror the midir callback: parse the raw bytes and push the event
    // onto the same kind of bounded channel the engine control thread
    // drains. A CC and a note should both arrive, in order; a malformed
    // message should produce nothing.
    let (tx, rx) = crossbeam_channel::bounded::<LiveControlEvent>(1024);
    for raw in [
        [0xB0, 7, 64].as_slice(),
        [0x90, 60, 100].as_slice(),
        [0xF8].as_slice(), // MIDI clock tick — not a control message.
    ] {
        if let Some(event) = parse_control_event_for_test(raw) {
            tx.try_send(event).unwrap();
        }
    }

    let drained: Vec<LiveControlEvent> = rx.try_iter().collect();
    assert_eq!(drained.len(), 2, "clock tick must not reach the channel");
    assert!(matches!(drained[0], LiveControlEvent::Cc { cc: 7, .. }));
    assert!(matches!(drained[1], LiveControlEvent::Note { note: 60, .. }));
}
