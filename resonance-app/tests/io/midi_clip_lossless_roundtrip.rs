//! MIDI clip notes survive save + reload exactly (code review STATE-06).
//!
//! Clips were saved only as Standard MIDI Files. The reader pairs one
//! pending note-on per pitch, so two overlapping same-pitch notes came back
//! as one wrong note; a velocity under ~1/254 encoded as a NoteOn with
//! velocity 0, which MIDI reads as a note-off, so the note vanished; every
//! velocity was quantised to 1/127; and same-tick notes were re-sorted by
//! pitch. Vocal lyrics are indexed by note position, so each of these also
//! shifted the lyrics. The notes now ride `project.json` losslessly.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use resonance_app::project::{self, ProjectFile, ProjectMidiClip};
use resonance_audio::types::MidiNote;

fn temp_project() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "resonance_midi_lossless_{}_{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create project dir");
    dir
}

fn note(note: u8, velocity: f32, start_tick: u64, duration_ticks: u64) -> MidiNote {
    MidiNote {
        note,
        velocity,
        start_tick,
        duration_ticks,
    }
}

#[test]
fn overlapping_quiet_and_same_tick_notes_round_trip_exactly() {
    let dir = temp_project();
    let notes = vec![
        // Two overlapping C4s.
        note(60, 0.8, 0, 960),
        note(60, 0.7, 480, 960),
        // Same start, higher pitch first — the order lyrics are keyed by.
        note(67, 0.5, 1920, 240),
        note(64, 0.33, 1920, 240),
        // So quiet it encodes as a note-off in MIDI.
        note(72, 0.001, 2400, 120),
    ];
    let project = ProjectFile {
        midi_clips: vec![ProjectMidiClip {
            id: 3,
            track_id: 1,
            start_sample: 0,
            duration_ticks: 3840,
            name: "clip".into(),
            trim_start_ticks: 0,
            trim_end_ticks: 0,
            midi_file: "midi/clip_3.mid".into(),
            vocal_lyrics: vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
            notes: None,
        }],
        ..ProjectFile::default()
    };

    project::save_project(&dir, &project, &[], &[(3, notes.clone())]).expect("save");
    let loaded = project::load_project(&dir).expect("load");
    let _ = std::fs::remove_dir_all(&dir);

    let mut by_id: HashMap<u64, Vec<MidiNote>> = loaded.midi_notes;
    let back = by_id.remove(&3).expect("clip notes loaded");
    assert!(
        resonance_app::update::project_io::replay_diff::midi_notes_equal(&notes, &back),
        "notes must round-trip field by field and in order:\n saved  {notes:?}\n loaded {back:?}"
    );
}
