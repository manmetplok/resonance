//! Unit coverage for the global chord-track data model (epic #33,
//! doc #168, todo #439): sort invariants on insert, region/key lookups,
//! and the undo snapshot round-trip via `UndoExtras`.

use resonance_app::chord_track::{ChordRegion, ChordTrack, KeyChange};
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_music_theory::{Chord, ChordQuality, Mode, PitchClass, Scale};

fn region(id: u64, start: u64, end: u64) -> ChordRegion {
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

// ---------------- Region sort invariant ----------------

#[test]
fn insert_region_keeps_sorted_by_start() {
    let mut track = ChordTrack::new();
    // Insert out of order.
    track.insert_region(region(1, 200, 300));
    track.insert_region(region(2, 0, 100));
    track.insert_region(region(3, 100, 200));

    let starts: Vec<u64> = track.regions.iter().map(|r| r.start_sample).collect();
    assert_eq!(starts, vec![0, 100, 200], "regions must stay sorted");
    let ids: Vec<u64> = track.regions.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![2, 3, 1]);
}

#[test]
fn region_at_finds_spanning_region_half_open() {
    let mut track = ChordTrack::new();
    track.insert_region(region(1, 0, 100));
    track.insert_region(region(2, 100, 200));

    assert_eq!(track.region_at(0).map(|r| r.id), Some(1));
    assert_eq!(track.region_at(99).map(|r| r.id), Some(1));
    // Boundary is exclusive on the end, inclusive on the start.
    assert_eq!(track.region_at(100).map(|r| r.id), Some(2));
    assert_eq!(track.region_at(199).map(|r| r.id), Some(2));
    // Past the last region's end → no match.
    assert_eq!(track.region_at(200).map(|r| r.id), None);
}

#[test]
fn region_at_returns_none_in_gap() {
    let mut track = ChordTrack::new();
    track.insert_region(region(1, 0, 100));
    track.insert_region(region(2, 200, 300));
    assert_eq!(track.region_at(150).map(|r| r.id), None);
}

#[test]
fn remove_region_returns_and_drops_it() {
    let mut track = ChordTrack::new();
    track.insert_region(region(1, 0, 100));
    track.insert_region(region(2, 100, 200));

    let removed = track.remove_region(1).expect("region 1 present");
    assert_eq!(removed.id, 1);
    assert_eq!(track.regions.len(), 1);
    assert_eq!(track.regions[0].id, 2);
    assert!(track.remove_region(99).is_none(), "absent id is a no-op");
}

#[test]
fn resort_restores_invariant_after_inplace_move() {
    let mut track = ChordTrack::new();
    track.insert_region(region(1, 0, 100));
    track.insert_region(region(2, 100, 200));

    // Drag region 1 past region 2 in place, breaking the sort.
    track.region_mut(1).unwrap().start_sample = 300;
    track.resort();

    let ids: Vec<u64> = track.regions.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![2, 1], "resort must reorder by start_sample");
}

// ---------------- Key context ----------------

#[test]
fn song_key_is_first_key_change() {
    let mut track = ChordTrack::new();
    assert_eq!(track.song_key(), None, "empty track has no key");

    track.insert_key_change(key_change(2, 48_000, PitchClass::G, Mode::Mixolydian));
    track.insert_key_change(key_change(1, 0, PitchClass::C, Mode::Major));

    assert_eq!(track.song_key(), Some(Scale::new(PitchClass::C, Mode::Major)));
    // The first key_changes entry (lowest position) is the song key.
    assert_eq!(track.key_changes[0].id, 1);
}

#[test]
fn key_at_resolves_active_scale() {
    let mut track = ChordTrack::new();
    track.insert_key_change(key_change(1, 0, PitchClass::C, Mode::Major));
    track.insert_key_change(key_change(2, 1000, PitchClass::A, Mode::Minor));

    // Before/at the first change → song key.
    assert_eq!(track.key_at(0), Some(Scale::new(PitchClass::C, Mode::Major)));
    assert_eq!(track.key_at(999), Some(Scale::new(PitchClass::C, Mode::Major)));
    // At/after the second change → the new key.
    assert_eq!(track.key_at(1000), Some(Scale::new(PitchClass::A, Mode::Minor)));
    assert_eq!(track.key_at(5000), Some(Scale::new(PitchClass::A, Mode::Minor)));
}

#[test]
fn key_at_before_first_change_uses_song_key() {
    let mut track = ChordTrack::new();
    // A single change positioned later than 0 still covers earlier
    // positions as the song key.
    track.insert_key_change(key_change(1, 5000, PitchClass::D, Mode::Dorian));
    assert_eq!(track.key_at(0), Some(Scale::new(PitchClass::D, Mode::Dorian)));
}

#[test]
fn remove_key_change_drops_it() {
    let mut track = ChordTrack::new();
    track.insert_key_change(key_change(1, 0, PitchClass::C, Mode::Major));
    track.insert_key_change(key_change(2, 1000, PitchClass::A, Mode::Minor));

    track.remove_key_change(2).expect("key 2 present");
    assert_eq!(track.key_changes.len(), 1);
    assert_eq!(track.song_key(), Some(Scale::new(PitchClass::C, Mode::Major)));
}

// ---------------- Undo round-trip ----------------

/// The chord track is declarative app state captured in the undo
/// snapshot's `extras` (it isn't part of `ProjectFile` yet). A snapshot
/// taken before an edit must restore the pre-edit track wholesale.
#[test]
fn chord_track_survives_undo_snapshot_round_trip() {
    let _ = STARTUP_TAB.set(resonance_app::state::ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();

    // Stage an initial progression + key.
    {
        let track = app.test_chord_track_mut();
        track.insert_key_change(key_change(1, 0, PitchClass::C, Mode::Major));
        track.insert_region(region(2, 0, 96_000));
    }

    // Capture the pre-edit state (what an undo would restore to).
    let snapshot = app.test_snapshot_for_undo();
    assert_eq!(
        snapshot.extras.chord_track.regions.len(),
        1,
        "snapshot must carry the chord track in its extras"
    );

    // Mutate further — add a region the undo should discard.
    app.test_chord_track_mut()
        .insert_region(region(3, 96_000, 192_000));
    assert_eq!(app.test_chord_track().regions.len(), 2);

    // Restore via the extras-apply path (slow-path undo).
    app.test_finalize_undo_restore(snapshot.extras);

    let restored = app.test_chord_track();
    assert_eq!(
        restored.regions.len(),
        1,
        "undo must drop the region added after the snapshot"
    );
    assert_eq!(restored.regions[0].id, 2);
    assert_eq!(restored.song_key(), Some(Scale::new(PitchClass::C, Mode::Major)));
}
