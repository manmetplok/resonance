use resonance_audio::midi_io::{read_midi_file, write_midi_file};
use resonance_audio::MidiNote;

fn tempdir() -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "resonance-midi-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&base).unwrap();
    base
}

#[test]
fn round_trip_notes() {
    let dir = tempdir();
    let path = dir.join("clip.mid");
    let input = vec![
        MidiNote {
            note: 60,
            velocity: 0.8,
            start_tick: 0,
            duration_ticks: 480,
        },
        MidiNote {
            note: 64,
            velocity: 0.5,
            start_tick: 240,
            duration_ticks: 240,
        },
        MidiNote {
            note: 67,
            velocity: 1.0,
            start_tick: 480,
            duration_ticks: 960,
        },
    ];
    write_midi_file(&path, &input).unwrap();
    let out = read_midi_file(&path).unwrap();
    assert_eq!(out.len(), input.len());
    for (a, b) in out.iter().zip(input.iter()) {
        assert_eq!(a.note, b.note);
        assert_eq!(a.start_tick, b.start_tick);
        assert_eq!(a.duration_ticks, b.duration_ticks);
        // Velocity round-trips through u7 so allow 1/127 slack.
        assert!((a.velocity - b.velocity).abs() <= 1.0 / 127.0 + 1e-6);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
