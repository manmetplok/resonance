//! Integration tests for the `SetTrackPlaybackSource` command boundary
//! (doc #257, todo #1099): the engine-side track-field update plus the
//! `TrackPlaybackSourceChanged` echo the app mirror rebuilds its state
//! from. Drives the pure in-place handler the dispatch arm delegates to
//! (the same pattern as the `external_instrument.rs` handlers) — the
//! full engine command loop needs a live audio device and so isn't
//! exercised headless.

use crossbeam_channel::unbounded;
use indexmap::IndexMap;

use resonance_audio::set_track_playback_source_in_place;
use resonance_audio::types::*;
use resonance_common::PlaybackSource;

#[test]
fn sets_track_field_and_echoes_event() {
    let mut tracks = IndexMap::new();
    tracks.insert(1, Track::new(1, "ext".into()));
    let (tx, rx) = unbounded();

    set_track_playback_source_in_place(&tracks, &tx, 1, PlaybackSource::Recorded);

    assert_eq!(tracks[&1].playback_source(), PlaybackSource::Recorded);
    match rx.try_recv() {
        Ok(AudioEvent::TrackPlaybackSourceChanged { track_id, source }) => {
            assert_eq!(track_id, 1);
            assert_eq!(source, PlaybackSource::Recorded);
        }
        other => panic!("expected TrackPlaybackSourceChanged, got {other:?}"),
    }
    assert!(rx.try_recv().is_err(), "exactly one echo per command");
}

#[test]
fn round_trips_back_to_live() {
    let mut tracks = IndexMap::new();
    tracks.insert(1, Track::new(1, "ext".into()));
    let (tx, rx) = unbounded();

    set_track_playback_source_in_place(&tracks, &tx, 1, PlaybackSource::Recorded);
    set_track_playback_source_in_place(&tracks, &tx, 1, PlaybackSource::Live);

    assert_eq!(tracks[&1].playback_source(), PlaybackSource::Live);
    let events: Vec<_> = rx.try_iter().collect();
    assert_eq!(events.len(), 2);
    match &events[1] {
        AudioEvent::TrackPlaybackSourceChanged { track_id, source } => {
            assert_eq!(*track_id, 1);
            assert_eq!(*source, PlaybackSource::Live);
        }
        other => panic!("expected TrackPlaybackSourceChanged, got {other:?}"),
    }
}

/// Missing lookup ⇒ no event: an unknown track changes nothing and
/// echoes nothing, so the app mirror never records a mode the engine
/// didn't apply.
#[test]
fn unknown_track_is_a_silent_noop() {
    let mut tracks = IndexMap::new();
    tracks.insert(1, Track::new(1, "ext".into()));
    let (tx, rx) = unbounded();

    set_track_playback_source_in_place(&tracks, &tx, 99, PlaybackSource::Recorded);

    assert_eq!(tracks[&1].playback_source(), PlaybackSource::Live);
    assert!(rx.try_recv().is_err());
}

/// The default mode on a fresh track is `Live` — exactly the pre-mode
/// behaviour.
#[test]
fn default_playback_source_is_live() {
    let track = Track::new(1, "ext".into());
    assert_eq!(track.playback_source(), PlaybackSource::Live);
    assert_eq!(PlaybackSource::default(), PlaybackSource::Live);
}
