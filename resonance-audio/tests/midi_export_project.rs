//! Format-1 project export: conductor track with tempo / time-signature
//! meta events plus one named track per source track. Parses the output
//! back with `midly` and asserts meta events land at the expected ticks
//! and that notes round-trip.

use midly::{MetaMessage, Smf, Timing, TrackEvent, TrackEventKind};
use resonance_audio::midi_io::{notes_from_track, write_midi_project, MidiTrackSource};
use resonance_audio::types::{SignaturePoint, TempoPoint};
use resonance_audio::{MidiNote, TempoMap};

const TPQN: u64 = 480;

fn tempdir() -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "resonance-midi-export-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn note(n: u8, start: u64, dur: u64) -> MidiNote {
    MidiNote {
        note: n,
        velocity: 0.8,
        start_tick: start,
        duration_ticks: dur,
    }
}

/// Absolute-tick list of meta events on a parsed track.
fn meta_events<'a>(track: &'a [TrackEvent<'a>]) -> Vec<(u64, MetaMessage<'a>)> {
    let mut out = Vec::new();
    let mut abs = 0u64;
    for ev in track {
        abs += u32::from(ev.delta) as u64;
        if let TrackEventKind::Meta(m) = ev.kind {
            out.push((abs, m));
        }
    }
    out
}

fn track_name(track: &[TrackEvent]) -> Option<String> {
    track.iter().find_map(|ev| match ev.kind {
        TrackEventKind::Meta(MetaMessage::TrackName(bytes)) => {
            Some(String::from_utf8_lossy(bytes).into_owned())
        }
        _ => None,
    })
}

#[test]
fn conductor_tempo_and_time_sig_at_expected_ticks() {
    let dir = tempdir();
    let path = dir.join("project.mid");

    // 120 BPM 4/4, tempo step to 140 at bar 4, signature change to 3/4
    // at bar 2. Tick of bar 2 = (4+4)*480 = 3840; bar 4 = (4+4+3+3)*480
    // = 6720.
    let mut tm = TempoMap::default();
    tm.bpm = 120.0;
    tm.numerator = 4;
    tm.denominator = 4;
    tm.tempo_points = vec![
        TempoPoint { bar: 0, bpm: 120.0 },
        TempoPoint { bar: 4, bpm: 140.0 },
    ];
    tm.signature_points = vec![
        SignaturePoint {
            bar: 0,
            numerator: 4,
            denominator: 4,
        },
        SignaturePoint {
            bar: 2,
            numerator: 3,
            denominator: 4,
        },
    ];

    let notes = vec![note(60, 0, TPQN)];
    write_midi_project(&path, &tm, &[MidiTrackSource { name: "Lead", notes: &notes }]).unwrap();

    let bytes = std::fs::read(&path).unwrap();
    let smf = Smf::parse(&bytes).unwrap();

    // Header: Format 1, metrical timing at TPQN.
    assert_eq!(smf.header.format, midly::Format::Parallel);
    assert_eq!(smf.header.timing, Timing::Metrical(midly::num::u15::new(TPQN as u16)));

    // Conductor + one note track.
    assert_eq!(smf.tracks.len(), 2);

    let conductor = meta_events(&smf.tracks[0]);

    // Tempo events: 120 BPM (500000 us) at tick 0; 140 BPM (~428571) at
    // tick 6720.
    let tempos: Vec<(u64, u32)> = conductor
        .iter()
        .filter_map(|(t, m)| match m {
            MetaMessage::Tempo(us) => Some((*t, u32::from(*us))),
            _ => None,
        })
        .collect();
    assert_eq!(tempos, vec![(0, 500_000), (6720, 428_571)]);

    // Time-signature events: 4/4 at tick 0; 3/4 at tick 3840. Denominator
    // is stored as a power of two (4 -> 2).
    let sigs: Vec<(u64, u8, u8)> = conductor
        .iter()
        .filter_map(|(t, m)| match m {
            MetaMessage::TimeSignature(num, den_pow2, _, _) => Some((*t, *num, *den_pow2)),
            _ => None,
        })
        .collect();
    assert_eq!(sigs, vec![(0, 4, 2), (3840, 3, 2)]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn multi_track_lists_names_and_round_trips_notes() {
    let dir = tempdir();
    let path = dir.join("multi.mid");

    let tm = TempoMap::default();
    let lead = vec![note(60, 0, TPQN), note(64, TPQN, TPQN)];
    let bass = vec![note(36, 0, 2 * TPQN)];

    write_midi_project(
        &path,
        &tm,
        &[
            MidiTrackSource { name: "Lead", notes: &lead },
            MidiTrackSource { name: "Bass", notes: &bass },
        ],
    )
    .unwrap();

    let bytes = std::fs::read(&path).unwrap();
    let smf = Smf::parse(&bytes).unwrap();

    // Conductor + two source tracks.
    assert_eq!(smf.tracks.len(), 3);

    // Conductor of a default tempo map: one tempo + one time signature.
    let conductor = meta_events(&smf.tracks[0]);
    assert!(conductor
        .iter()
        .any(|(t, m)| *t == 0 && matches!(m, MetaMessage::Tempo(_))));
    assert!(conductor
        .iter()
        .any(|(t, m)| *t == 0 && matches!(m, MetaMessage::TimeSignature(..))));

    // Track names in order.
    assert_eq!(track_name(&smf.tracks[1]).as_deref(), Some("Lead"));
    assert_eq!(track_name(&smf.tracks[2]).as_deref(), Some("Bass"));

    // Notes round-trip per track.
    let lead_out = notes_from_track(&smf.tracks[1]);
    assert_eq!(lead_out.len(), lead.len());
    for (a, b) in lead_out.iter().zip(lead.iter()) {
        assert_eq!(a.note, b.note);
        assert_eq!(a.start_tick, b.start_tick);
        assert_eq!(a.duration_ticks, b.duration_ticks);
    }

    let bass_out = notes_from_track(&smf.tracks[2]);
    assert_eq!(bass_out.len(), 1);
    assert_eq!(bass_out[0].note, 36);
    assert_eq!(bass_out[0].start_tick, 0);
    assert_eq!(bass_out[0].duration_ticks, 2 * TPQN);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn signature_point_before_first_bar_uses_first_value() {
    // Mirrors TempoMap: when the first signature point is past bar 0,
    // bars before it inherit that first point's value. A 3/4 point at
    // bar 1 means bar 0 is also 3/4, so bar 2's tick is 3*2 beats * 480.
    let dir = tempdir();
    let path = dir.join("sig.mid");

    let mut tm = TempoMap::default();
    tm.signature_points = vec![
        SignaturePoint {
            bar: 1,
            numerator: 3,
            denominator: 4,
        },
        SignaturePoint {
            bar: 3,
            numerator: 5,
            denominator: 8,
        },
    ];

    write_midi_project(&path, &tm, &[]).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let smf = Smf::parse(&bytes).unwrap();

    let conductor = meta_events(&smf.tracks[0]);
    let sigs: Vec<(u64, u8, u8)> = conductor
        .iter()
        .filter_map(|(t, m)| match m {
            MetaMessage::TimeSignature(num, den_pow2, _, _) => Some((*t, *num, *den_pow2)),
            _ => None,
        })
        .collect();

    // tick 0: 3/4 (first point's value applies from bar 0).
    // bar 1 tick = 3*480 = 1440: 3/4 again (explicit point).
    // bar 3 tick = (3+3+3)*480 = 4320: 5/8 (denominator 8 -> pow2 3).
    assert_eq!(sigs, vec![(0, 3, 2), (1440, 3, 2), (4320, 5, 3)]);

    let _ = std::fs::remove_dir_all(&dir);
}
