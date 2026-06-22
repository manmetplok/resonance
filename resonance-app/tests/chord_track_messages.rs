//! Update-handler coverage for the global chord track (epic #33,
//! doc #168, todo #441): the `ChordTrackMessage` group + its handlers
//! and undo wiring.
//!
//! Snapping is exercised by the handlers but made deterministic here by
//! pinning the Arrange zoom to `0.0` — `snap_sample_to_grid_tempo`
//! returns the input unchanged for a non-positive zoom, so every sample
//! position below is exact. The clamping behaviour (`MoveStart`/`SetEnd`
//! staying sorted and non-overlapping) is asserted independently of the
//! tempo grid by driving inputs past the neighbouring bounds.

use resonance_app::chord_track::{ChordRegion, KeyChange};
use resonance_app::message::{ChordTrackMessage, Message, TransportMessage};
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_music_theory::{parse_chord, Chord, ChordQuality, Mode, PitchClass, Scale};

/// A chord-track app with an active project (so chord messages aren't
/// gated) and snapping disabled (zoom 0 → identity snap).
fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    // A saved path is required for undo recording (`can_record_undo`).
    app.test_set_project_path(Some(std::path::PathBuf::from("/tmp/chord-test.rprj")));
    app.test_set_arrange_zoom(0.0);
    app
}

fn cmaj_region(id: u64, start: u64, end: u64) -> ChordRegion {
    ChordRegion {
        id,
        chord: Chord::new(PitchClass::C, ChordQuality::Maj),
        start_sample: start,
        end_sample: end,
        pinned: false,
    }
}

fn key_change(id: u64, start: u64, root: PitchClass, mode: Mode) -> KeyChange {
    KeyChange {
        id,
        start_sample: start,
        scale: Scale::new(root, mode),
    }
}

fn send(app: &mut Resonance, m: ChordTrackMessage) {
    let _ = app.update(Message::ChordTrack(m));
}

// ---------------- Add ----------------

#[test]
fn add_at_playhead_inserts_default_c_major_region() {
    let mut app = app();
    // Playhead defaults to 0; an empty track.
    send(&mut app, ChordTrackMessage::AddAtPlayhead);

    let track = app.test_chord_track();
    assert_eq!(track.regions.len(), 1, "one region added");
    let r = &track.regions[0];
    assert_eq!(r.start_sample, 0);
    assert!(r.end_sample > 0, "default region has a positive length");
    assert_eq!(r.chord, Chord::new(PitchClass::C, ChordQuality::Maj));
    assert!(!r.pinned);
}

#[test]
fn add_at_playhead_skips_when_region_already_starts_there() {
    let mut app = app();
    app.test_chord_track_mut().insert_region(cmaj_region(1, 0, 100));
    // Playhead 0 coincides with the existing region's start.
    send(&mut app, ChordTrackMessage::AddAtPlayhead);
    assert_eq!(
        app.test_chord_track().regions.len(),
        1,
        "no duplicate region at the same start"
    );
}

#[test]
fn add_at_playhead_splits_the_region_under_the_playhead() {
    let mut app = app();
    app.test_chord_track_mut().insert_region(cmaj_region(1, 0, 100));
    // Move the playhead inside the region, then add.
    let _ = app.update(Message::Transport(TransportMessage::SeekToSample(50)));
    send(&mut app, ChordTrackMessage::AddAtPlayhead);

    let regions = &app.test_chord_track().regions;
    assert_eq!(regions.len(), 2, "region split into two");
    assert_eq!(regions[0].start_sample, 0);
    assert_eq!(regions[0].end_sample, 50, "host region shortened to playhead");
    assert_eq!(regions[1].start_sample, 50);
    assert_eq!(regions[1].end_sample, 100, "new region inherits the tail");
}

#[test]
fn add_at_playhead_seeds_tonic_from_song_key() {
    let mut app = app();
    // Song key A minor: a region added at the playhead should default to
    // the tonic triad (Am), not a hardcoded C major (doc #168).
    app.test_chord_track_mut()
        .insert_key_change(key_change(1, 0, PitchClass::A, Mode::Minor));
    send(&mut app, ChordTrackMessage::AddAtPlayhead);

    let track = app.test_chord_track();
    assert_eq!(track.regions.len(), 1);
    assert_eq!(
        track.regions[0].chord,
        Chord::new(PitchClass::A, ChordQuality::Min),
        "default chord is the tonic of the song key"
    );
}

#[test]
fn add_at_playhead_seeds_tonic_from_key_at_position() {
    let mut app = app();
    // Song key C major, then a G Lydian change at 200. The playhead sits
    // past the change, so the new region picks up the key in effect there
    // (G major triad), exercising `key_at` rather than just the song key.
    app.test_chord_track_mut()
        .insert_key_change(key_change(1, 0, PitchClass::C, Mode::Major));
    app.test_chord_track_mut()
        .insert_key_change(key_change(2, 200, PitchClass::G, Mode::Lydian));
    let _ = app.update(Message::Transport(TransportMessage::SeekToSample(300)));
    send(&mut app, ChordTrackMessage::AddAtPlayhead);

    let region = app
        .test_chord_track()
        .regions
        .iter()
        .find(|r| r.start_sample == 300)
        .expect("region added at the playhead");
    assert_eq!(
        region.chord,
        Chord::new(PitchClass::G, ChordQuality::Maj),
        "Lydian seeds a major tonic from the key in effect at the start"
    );
}

#[test]
fn add_region_parses_symbol_and_inserts() {
    let mut app = app();
    send(
        &mut app,
        ChordTrackMessage::AddRegion {
            start_sample: 0,
            end_sample: 88_200,
            symbol: "Am7".to_string(),
        },
    );
    let track = app.test_chord_track();
    assert_eq!(track.regions.len(), 1);
    assert_eq!(track.regions[0].chord, parse_chord("Am7").unwrap());
    assert!(track.last_error.is_none());
}

#[test]
fn add_region_with_bad_symbol_sets_error_and_inserts_nothing() {
    let mut app = app();
    send(
        &mut app,
        ChordTrackMessage::AddRegion {
            start_sample: 0,
            end_sample: 88_200,
            symbol: "H7".to_string(), // 'H' is not a note name
        },
    );
    let track = app.test_chord_track();
    assert!(track.regions.is_empty(), "bad symbol adds no region");
    assert!(track.last_error.is_some(), "parse error surfaced");
}

#[test]
fn add_region_caps_end_at_next_region() {
    let mut app = app();
    app.test_chord_track_mut()
        .insert_region(cmaj_region(1, 1_000, 2_000));
    send(
        &mut app,
        ChordTrackMessage::AddRegion {
            start_sample: 0,
            end_sample: 5_000, // would overrun region 1
            symbol: "C".to_string(),
        },
    );
    let regions = &app.test_chord_track().regions;
    assert_eq!(regions.len(), 2);
    // The freshly-added region is the one starting at 0.
    let added = regions.iter().find(|r| r.start_sample == 0).unwrap();
    assert_eq!(added.end_sample, 1_000, "end capped at the next region start");
}

// ---------------- SetSymbol ----------------

#[test]
fn set_symbol_updates_chord_and_clears_error() {
    let mut app = app();
    app.test_chord_track_mut().insert_region(cmaj_region(1, 0, 100));
    app.test_chord_track_mut().last_error = Some("stale".to_string());

    send(
        &mut app,
        ChordTrackMessage::SetSymbol {
            id: 1,
            symbol: "Dm".to_string(),
        },
    );
    let track = app.test_chord_track();
    assert_eq!(track.region(1).unwrap().chord, parse_chord("Dm").unwrap());
    assert!(track.last_error.is_none(), "successful edit clears last_error");
}

#[test]
fn set_symbol_with_bad_input_keeps_region_and_sets_error() {
    let mut app = app();
    app.test_chord_track_mut().insert_region(cmaj_region(1, 0, 100));
    send(
        &mut app,
        ChordTrackMessage::SetSymbol {
            id: 1,
            symbol: "zzz".to_string(),
        },
    );
    let track = app.test_chord_track();
    assert_eq!(
        track.region(1).unwrap().chord,
        Chord::new(PitchClass::C, ChordQuality::Maj),
        "region unchanged on parse error"
    );
    assert!(track.last_error.is_some());
}

// ---------------- Move / SetEnd clamping ----------------

#[test]
fn move_start_clamps_to_previous_region_end() {
    let mut app = app();
    app.test_chord_track_mut().insert_region(cmaj_region(1, 0, 100));
    app.test_chord_track_mut()
        .insert_region(cmaj_region(2, 150, 250));
    // Try to drag region 2's start back to 0 — must clamp at region 1's end.
    send(&mut app, ChordTrackMessage::MoveStart { id: 2, sample: 0 });

    let regions = &app.test_chord_track().regions;
    let r2 = regions.iter().find(|r| r.id == 2).unwrap();
    assert_eq!(r2.start_sample, 100, "clamped to previous region end");
    // Still sorted & non-overlapping.
    assert!(regions[0].end_sample <= regions[1].start_sample);
}

#[test]
fn move_start_clamps_before_own_end() {
    let mut app = app();
    app.test_chord_track_mut()
        .insert_region(cmaj_region(1, 150, 250));
    send(
        &mut app,
        ChordTrackMessage::MoveStart {
            id: 1,
            sample: 1_000_000,
        },
    );
    let r = app.test_chord_track().region(1).unwrap();
    assert_eq!(r.start_sample, 249, "start clamped to just before own end");
    assert!(r.start_sample < r.end_sample);
}

#[test]
fn set_end_clamps_to_next_region_start() {
    let mut app = app();
    app.test_chord_track_mut().insert_region(cmaj_region(1, 0, 100));
    app.test_chord_track_mut()
        .insert_region(cmaj_region(2, 200, 300));
    send(
        &mut app,
        ChordTrackMessage::SetEnd {
            id: 1,
            sample: 1_000_000,
        },
    );
    let r = app.test_chord_track().region(1).unwrap();
    assert_eq!(r.end_sample, 200, "end capped at next region start");
}

#[test]
fn set_end_clamps_after_own_start() {
    let mut app = app();
    app.test_chord_track_mut()
        .insert_region(cmaj_region(1, 100, 200));
    send(&mut app, ChordTrackMessage::SetEnd { id: 1, sample: 0 });
    let r = app.test_chord_track().region(1).unwrap();
    assert_eq!(r.end_sample, 101, "end clamped to just after own start");
    assert!(r.end_sample > r.start_sample);
}

// ---------------- Delete / Pin ----------------

#[test]
fn delete_removes_region() {
    let mut app = app();
    app.test_chord_track_mut().insert_region(cmaj_region(1, 0, 100));
    send(&mut app, ChordTrackMessage::Delete { id: 1 });
    assert!(app.test_chord_track().regions.is_empty());
}

#[test]
fn toggle_pin_flips_the_flag() {
    let mut app = app();
    app.test_chord_track_mut().insert_region(cmaj_region(1, 0, 100));
    send(&mut app, ChordTrackMessage::TogglePin { id: 1 });
    assert!(app.test_chord_track().region(1).unwrap().pinned);
    send(&mut app, ChordTrackMessage::TogglePin { id: 1 });
    assert!(!app.test_chord_track().region(1).unwrap().pinned);
}

// ---------------- Key context ----------------

#[test]
fn set_song_key_inserts_then_retunes_first_entry() {
    let mut app = app();
    send(
        &mut app,
        ChordTrackMessage::SetSongKey {
            scale: Scale::new(PitchClass::C, Mode::Major),
        },
    );
    {
        let track = app.test_chord_track();
        assert_eq!(track.key_changes.len(), 1);
        assert_eq!(track.key_changes[0].start_sample, 0);
        assert_eq!(track.song_key(), Some(Scale::new(PitchClass::C, Mode::Major)));
    }
    // Setting again retunes the same (earliest) entry, not a second one.
    send(
        &mut app,
        ChordTrackMessage::SetSongKey {
            scale: Scale::new(PitchClass::A, Mode::Minor),
        },
    );
    let track = app.test_chord_track();
    assert_eq!(track.key_changes.len(), 1, "still a single song key");
    assert_eq!(track.song_key(), Some(Scale::new(PitchClass::A, Mode::Minor)));
}

#[test]
fn insert_key_change_adds_then_retunes_at_same_position() {
    let mut app = app();
    send(
        &mut app,
        ChordTrackMessage::InsertKeyChange {
            sample: 100,
            scale: Scale::new(PitchClass::G, Mode::Major),
        },
    );
    assert_eq!(app.test_chord_track().key_changes.len(), 1);
    // A second insert at the same snapped position retunes in place.
    send(
        &mut app,
        ChordTrackMessage::InsertKeyChange {
            sample: 100,
            scale: Scale::new(PitchClass::D, Mode::Major),
        },
    );
    let track = app.test_chord_track();
    assert_eq!(track.key_changes.len(), 1, "no duplicate at the same sample");
    assert_eq!(track.key_changes[0].scale, Scale::new(PitchClass::D, Mode::Major));
}

#[test]
fn move_key_change_resorts() {
    let mut app = app();
    app.test_chord_track_mut()
        .insert_key_change(key_change(1, 0, PitchClass::C, Mode::Major));
    app.test_chord_track_mut()
        .insert_key_change(key_change(2, 200, PitchClass::G, Mode::Major));
    // Move the first past the second.
    send(&mut app, ChordTrackMessage::MoveKeyChange { id: 1, sample: 300 });

    let kc = &app.test_chord_track().key_changes;
    let starts: Vec<u64> = kc.iter().map(|k| k.start_sample).collect();
    let ids: Vec<u64> = kc.iter().map(|k| k.id).collect();
    assert_eq!(starts, vec![200, 300], "key changes stay sorted by start");
    assert_eq!(ids, vec![2, 1]);
}

#[test]
fn delete_key_change_removes_it() {
    let mut app = app();
    app.test_chord_track_mut()
        .insert_key_change(key_change(1, 0, PitchClass::C, Mode::Major));
    app.test_chord_track_mut()
        .insert_key_change(key_change(2, 200, PitchClass::G, Mode::Major));
    send(&mut app, ChordTrackMessage::DeleteKeyChange { id: 1 });

    let kc = &app.test_chord_track().key_changes;
    assert_eq!(kc.len(), 1);
    assert_eq!(kc[0].id, 2);
}

// ---------------- Undo wiring ----------------

#[test]
fn each_edit_pushes_one_undo_entry_and_undo_redo_restores() {
    let mut app = app();
    // Edit 1: add a region.
    send(&mut app, ChordTrackMessage::AddAtPlayhead);
    assert_eq!(app.test_chord_track().regions.len(), 1);
    // Edit 2: set the song key.
    send(
        &mut app,
        ChordTrackMessage::SetSongKey {
            scale: Scale::new(PitchClass::C, Mode::Major),
        },
    );
    assert_eq!(app.test_chord_track().key_changes.len(), 1);

    // One undo rewinds exactly the song-key edit.
    let _ = app.update(Message::Undo);
    assert_eq!(app.test_chord_track().key_changes.len(), 0, "song key undone");
    assert_eq!(app.test_chord_track().regions.len(), 1, "region still present");

    // A second undo rewinds the add.
    let _ = app.update(Message::Undo);
    assert!(app.test_chord_track().regions.is_empty(), "region undone");

    // Redo walks forward again, one entry at a time.
    let _ = app.update(Message::Redo);
    assert_eq!(app.test_chord_track().regions.len(), 1, "region redone");
    let _ = app.update(Message::Redo);
    assert_eq!(app.test_chord_track().key_changes.len(), 1, "song key redone");
}
