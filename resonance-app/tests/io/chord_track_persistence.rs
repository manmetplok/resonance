//! The global chord track survives save → load (ARCH-01 A1-2 9a).
//!
//! Until `ProjectFile::chord_track` existed the track was undoable but
//! never written to disk, so every reload silently dropped the song's
//! harmony. Three facts pinned here: a saved project comes back with the
//! same regions and key changes through the real `save_project` /
//! `load_project` / `replay_loaded_project` chain; the transient parse
//! error banner is *not* persisted; and a legacy `project.json` without
//! the field loads with an empty track.

use std::path::PathBuf;

use resonance_app::chord_track::{ChordRegion, KeyChange};
use resonance_app::project::{load_project, save_project, ProjectFile};
use resonance_app::Resonance;
use resonance_music_theory::{Chord, ChordQuality, Mode, PitchClass, Scale};

fn project_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-chord-track-persist-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create project dir");
    dir
}

fn region(id: u64, chord: Chord, start: u64, end: u64, pinned: bool) -> ChordRegion {
    ChordRegion {
        id,
        chord,
        start_sample: start,
        end_sample: end,
        pinned,
    }
}

#[test]
fn chord_track_round_trips_through_save_and_load() {
    let dir = project_dir("round-trip");

    let (mut authored, _task) = Resonance::new_for_test();
    {
        let track = authored.test_chord_track_mut();
        track.insert_key_change(KeyChange {
            id: 1,
            start_sample: 0,
            scale: Scale::new(PitchClass::D, Mode::Minor),
        });
        track.insert_key_change(KeyChange {
            id: 2,
            start_sample: 192_000,
            scale: Scale::new(PitchClass::F, Mode::Major),
        });
        track.insert_region(region(
            10,
            Chord::new(PitchClass::D, ChordQuality::Min),
            0,
            96_000,
            true,
        ));
        track.insert_region(region(
            11,
            Chord::new(PitchClass::G, ChordQuality::Maj),
            96_000,
            192_000,
            false,
        ));
        // View feedback from a failed symbol parse: must not reach disk.
        track.last_error = Some("unparseable".to_string());
    }
    let expected = authored.test_chord_track().clone();

    let file = authored.test_build_project_file();
    assert_eq!(file.chord_track.regions.len(), 2, "regions serialized");
    assert_eq!(
        file.chord_track.key_changes.len(),
        2,
        "key changes serialized"
    );
    save_project(&dir, &file, &[], &[]).expect("save project");

    let json = std::fs::read_to_string(dir.join("project.json")).expect("read project.json");
    assert!(
        json.contains("\"chord_track\""),
        "chord track written to disk"
    );
    assert!(
        !json.contains("unparseable"),
        "the transient parse error is not project state"
    );

    let loaded = load_project(&dir).expect("load project");
    let (mut reopened, _task) = Resonance::new_for_test();
    reopened.test_replay_loaded_project_from(loaded);

    let restored = reopened.test_chord_track();
    assert_eq!(restored.regions, expected.regions, "regions survive reload");
    assert_eq!(
        restored.key_changes, expected.key_changes,
        "key changes survive reload"
    );
    assert_eq!(
        restored.song_key(),
        Some(Scale::new(PitchClass::D, Mode::Minor))
    );
    assert_eq!(
        restored.last_error, None,
        "reload starts with a clear banner"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_project_without_chord_track_loads_empty() {
    let mut json = serde_json::to_value(ProjectFile::default()).unwrap();
    json.as_object_mut().unwrap().remove("chord_track");

    let file: ProjectFile = serde_json::from_value(json).expect("legacy project deserializes");
    assert!(file.chord_track.regions.is_empty());
    assert!(file.chord_track.key_changes.is_empty());

    let (mut app, _task) = Resonance::new_for_test();
    app.test_replay_loaded_project(file);
    assert!(app.test_chord_track().is_empty());
    assert_eq!(app.test_chord_track().song_key(), None);
}
