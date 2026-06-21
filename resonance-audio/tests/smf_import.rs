//! Tests for the structured SMF importer (`parse_midi_file` /
//! `parse_smf_bytes` → `ImportedSmf`).

use midly::num::{u15, u24, u28, u4, u7};
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind,
};
use resonance_audio::midi_io::{parse_midi_file, parse_smf_bytes, SmfFormat};

/// Serialize an `Smf` made of `(delta, kind)` tracks to bytes. Each track
/// is terminated with an End-of-Track meta event.
fn encode(format: Format, ppq: u16, tracks: Vec<Vec<(u32, TrackEventKind<'_>)>>) -> Vec<u8> {
    let mut smf_tracks: Vec<Track> = Vec::new();
    for events in tracks {
        let mut track = Track::new();
        for (delta, kind) in events {
            track.push(TrackEvent {
                delta: u28::new(delta),
                kind,
            });
        }
        track.push(TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        });
        smf_tracks.push(track);
    }
    let smf = Smf {
        header: Header::new(format, Timing::Metrical(u15::new(ppq))),
        tracks: smf_tracks,
    };
    let mut buf = Vec::new();
    smf.write(&mut buf).expect("serialize smf");
    buf
}

fn note_on(ch: u8, key: u8, vel: u8) -> TrackEventKind<'static> {
    TrackEventKind::Midi {
        channel: u4::new(ch),
        message: MidiMessage::NoteOn {
            key: u7::new(key),
            vel: u7::new(vel),
        },
    }
}

fn note_off(ch: u8, key: u8) -> TrackEventKind<'static> {
    TrackEventKind::Midi {
        channel: u4::new(ch),
        message: MidiMessage::NoteOff {
            key: u7::new(key),
            vel: u7::new(0),
        },
    }
}

fn tempo(bpm: u32) -> TrackEventKind<'static> {
    let us = 60_000_000 / bpm;
    TrackEventKind::Meta(MetaMessage::Tempo(u24::new(us)))
}

/// `denom_pow` is the power-of-two exponent (2 → /4, 3 → /8).
fn time_sig(numer: u8, denom_pow: u8) -> TrackEventKind<'static> {
    TrackEventKind::Meta(MetaMessage::TimeSignature(numer, denom_pow, 24, 8))
}

#[test]
fn format0_single_track_multi_channel() {
    // PPQ 480 = engine PPQ, so ticks pass through unscaled. Two notes on
    // two different channels; both must be imported (not just channel 0).
    let bytes = encode(
        Format::SingleTrack,
        480,
        vec![vec![
            (0, note_on(0, 60, 100)),
            (0, note_on(1, 72, 80)),
            (480, note_off(0, 60)),
            (0, note_off(1, 72)),
        ]],
    );

    let smf = parse_smf_bytes(&bytes).unwrap();
    assert_eq!(smf.format, SmfFormat::Format0);
    assert_eq!(smf.source_ppq, 480);
    assert_eq!(smf.track_count, 1);

    let t = &smf.tracks[0];
    assert_eq!(t.note_count, 2);
    assert_eq!(t.channels, vec![0, 1]);
    assert_eq!(t.pitch_min, Some(60));
    assert_eq!(t.pitch_max, Some(72));
    assert!(!t.is_conductor);

    // Notes sorted by (start_tick, note); both start at 0.
    assert_eq!(t.notes[0].note, 60);
    assert_eq!(t.notes[0].start_tick, 0);
    assert_eq!(t.notes[0].duration_ticks, 480);
    assert_eq!(t.notes[1].note, 72);
    assert_eq!(t.notes[1].duration_ticks, 480);

    assert_eq!(smf.length_ticks, 480);
    assert_eq!(smf.length_bars, 1);
}

#[test]
fn format1_multi_track_with_conductor() {
    // Track 0: conductor (tempo + meter + name, no notes).
    // Tracks 1 & 2: instruments with notes.
    let bytes = encode(
        Format::Parallel,
        480,
        vec![
            vec![
                (0, TrackEventKind::Meta(MetaMessage::TrackName(b"Conductor"))),
                (0, time_sig(4, 2)),
                (0, tempo(120)),
            ],
            vec![
                (0, TrackEventKind::Meta(MetaMessage::TrackName(b"Bass"))),
                (0, note_on(0, 40, 100)),
                (960, note_off(0, 40)),
            ],
            vec![
                (0, note_on(2, 64, 90)),
                (480, note_off(2, 64)),
            ],
        ],
    );

    let smf = parse_smf_bytes(&bytes).unwrap();
    assert_eq!(smf.format, SmfFormat::Format1);
    assert_eq!(smf.track_count, 3);

    let cond = &smf.tracks[0];
    assert!(cond.is_conductor);
    assert_eq!(cond.note_count, 0);
    assert_eq!(cond.name.as_deref(), Some("Conductor"));

    let bass = &smf.tracks[1];
    assert!(!bass.is_conductor);
    assert_eq!(bass.name.as_deref(), Some("Bass"));
    assert_eq!(bass.note_count, 1);
    assert_eq!(bass.channels, vec![0]);

    let lead = &smf.tracks[2];
    assert_eq!(lead.name, None);
    assert_eq!(lead.channels, vec![2]);

    // Longest note end (960) → bar 0 only (4/4 bar is 1920).
    assert_eq!(smf.length_ticks, 960);
    assert_eq!(smf.length_bars, 1);
}

#[test]
fn ppq_scaling_to_engine_ticks() {
    // File PPQ 96 → engine 480 (×5).
    let bytes = encode(
        Format::SingleTrack,
        96,
        vec![vec![
            (48, note_on(0, 60, 100)), // start at an eighth note
            (48, note_off(0, 60)),     // quarter-note total span ends at 96
        ]],
    );
    let smf = parse_smf_bytes(&bytes).unwrap();
    assert_eq!(smf.source_ppq, 96);
    let n = &smf.tracks[0].notes[0];
    assert_eq!(n.start_tick, 240); // 48 * 480 / 96
    assert_eq!(n.duration_ticks, 240); // 48 * 480 / 96
}

#[test]
fn tick_scaling_rounds_half_up() {
    // PPQ 320: 1 tick → 1.5 engine ticks → rounds up to 2.
    let bytes = encode(
        Format::SingleTrack,
        320,
        vec![vec![(1, note_on(0, 60, 100)), (1, note_off(0, 60))]],
    );
    let smf = parse_smf_bytes(&bytes).unwrap();
    let n = &smf.tracks[0].notes[0];
    assert_eq!(n.start_tick, 2); // round_half_up(1 * 480 / 320 = 1.5) = 2
    assert_eq!(n.duration_ticks, 2);
}

#[test]
fn tempo_and_meter_changes_resolve_to_bars() {
    // 4 bars of 4/4 at 120bpm, then 6/8 at 140bpm. PPQ 480 (1:1 ticks).
    // bar 4 starts at 4 * 1920 = 7680 ticks.
    let bytes = encode(
        Format::Parallel,
        480,
        vec![vec![
            (0, time_sig(4, 2)),     // 4/4
            (0, tempo(120)),         // 120 bpm
            (7680, time_sig(6, 3)),  // → 6/8 at bar 4
            (0, tempo(140)),         // 140 bpm, same tick
        ]],
    );

    let smf = parse_smf_bytes(&bytes).unwrap();

    // BPM round-trips through integer µs-per-quarter, so allow slack.
    assert!((smf.tempo_min_bpm - 120.0).abs() < 0.01);
    assert!((smf.tempo_max_bpm - 140.0).abs() < 0.01);

    // Absolute-tick events.
    assert_eq!(smf.signature_events.len(), 2);
    assert_eq!(smf.signature_events[0].tick, 0);
    assert_eq!(smf.signature_events[0].numerator, 4);
    assert_eq!(smf.signature_events[0].denominator, 4);
    assert_eq!(smf.signature_events[1].tick, 7680);
    assert_eq!(smf.signature_events[1].numerator, 6);
    assert_eq!(smf.signature_events[1].denominator, 8);

    // Bar-indexed points ready for the TempoMap.
    assert_eq!(smf.signature_points[0].bar, 0);
    assert_eq!(smf.signature_points[1].bar, 4);
    assert_eq!(smf.tempo_points[0].bar, 0);
    assert!((smf.tempo_points[0].bpm - 120.0).abs() < 0.01);
    assert_eq!(smf.tempo_points[1].bar, 4);
    assert!((smf.tempo_points[1].bpm - 140.0).abs() < 0.01);
}

#[test]
fn track_name_falls_back_to_instrument_name() {
    let bytes = encode(
        Format::SingleTrack,
        480,
        vec![vec![
            (0, TrackEventKind::Meta(MetaMessage::InstrumentName(b"Strings"))),
            (0, note_on(0, 60, 100)),
            (480, note_off(0, 60)),
        ]],
    );
    let smf = parse_smf_bytes(&bytes).unwrap();
    assert_eq!(smf.tracks[0].name.as_deref(), Some("Strings"));
}

#[test]
fn format2_is_rejected() {
    let bytes = encode(Format::Sequential, 480, vec![vec![(0, note_on(0, 60, 100))]]);
    let err = parse_smf_bytes(&bytes).unwrap_err();
    assert!(err.contains("Format 2"), "unexpected error: {err}");
}

#[test]
fn corrupt_bytes_are_rejected() {
    let err = parse_smf_bytes(b"this is not a midi file").unwrap_err();
    assert!(err.contains("parse smf"), "unexpected error: {err}");
}

#[test]
fn parse_midi_file_reads_from_disk() {
    let bytes = encode(
        Format::SingleTrack,
        480,
        vec![vec![(0, note_on(0, 60, 100)), (480, note_off(0, 60))]],
    );
    let dir = std::env::temp_dir().join(format!(
        "resonance-smf-import-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("song.mid");
    std::fs::write(&path, &bytes).unwrap();

    let smf = parse_midi_file(&path).unwrap();
    assert_eq!(smf.track_count, 1);
    assert_eq!(smf.tracks[0].note_count, 1);

    let _ = std::fs::remove_dir_all(&dir);
}
