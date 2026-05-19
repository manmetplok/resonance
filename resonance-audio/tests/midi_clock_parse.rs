//! Unit tests for MIDI clock parser and tempo tracker.

use std::time::{Duration, Instant};

use resonance_audio::__test_support::{parse_clock_message, ClockTempoTracker, MidiClockEvent};

#[test]
fn parses_clock_pulse() {
    let now = Instant::now();
    let ev = parse_clock_message(&[0xF8], now).expect("parse F8");
    assert!(matches!(ev, MidiClockEvent::Clock { .. }));
}

#[test]
fn parses_start_continue_stop() {
    let now = Instant::now();
    assert!(matches!(
        parse_clock_message(&[0xFA], now),
        Some(MidiClockEvent::Start { .. })
    ));
    assert!(matches!(
        parse_clock_message(&[0xFB], now),
        Some(MidiClockEvent::Continue { .. })
    ));
    assert!(matches!(
        parse_clock_message(&[0xFC], now),
        Some(MidiClockEvent::Stop)
    ));
}

#[test]
fn parses_song_position_pointer() {
    // SPP: F2 + LSB + MSB, value = MSB << 7 | LSB. 256 = 0x100 → LSB=0, MSB=2.
    let now = Instant::now();
    let ev = parse_clock_message(&[0xF2, 0x00, 0x02], now).expect("parse SPP");
    match ev {
        MidiClockEvent::SongPosition { sixteenths } => assert_eq!(sixteenths, 256),
        other => panic!("expected SongPosition, got {other:?}"),
    }
}

#[test]
fn ignores_unrelated_status_bytes() {
    let now = Instant::now();
    // Note On — not a clock message.
    assert!(parse_clock_message(&[0x90, 60, 100], now).is_none());
}

#[test]
fn rejects_truncated_song_position() {
    let now = Instant::now();
    // SPP needs three bytes.
    assert!(parse_clock_message(&[0xF2, 0x00], now).is_none());
}

#[test]
fn tempo_tracker_returns_none_until_window_filled() {
    let mut t = ClockTempoTracker::new(4);
    let mut now = Instant::now();
    // Need window+1 samples before any reading is produced.
    for _ in 0..4 {
        assert!(t.observe(now).is_none());
        now += Duration::from_millis(20);
    }
    let bpm = t.observe(now).expect("BPM after window fills");
    // 20 ms per pulse * 24 pulses/quarter = 480 ms/quarter → 125 BPM.
    assert!((bpm - 125.0).abs() < 0.5, "expected ≈125 BPM, got {bpm}");
}

#[test]
fn tempo_tracker_recomputes_after_reset() {
    let mut t = ClockTempoTracker::new(4);
    let mut now = Instant::now();
    for _ in 0..5 {
        let _ = t.observe(now);
        now += Duration::from_millis(20);
    }
    t.reset();
    // After reset we should require a fresh window again.
    let mut now = Instant::now();
    for _ in 0..4 {
        assert!(t.observe(now).is_none());
        now += Duration::from_millis(10);
    }
    let bpm = t.observe(now).expect("BPM after window fills");
    // 10 ms per pulse * 24 = 240 ms/quarter → 250 BPM.
    assert!((bpm - 250.0).abs() < 1.0, "expected ≈250 BPM, got {bpm}");
}
