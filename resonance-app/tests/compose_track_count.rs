//! The Compose TRACKS group header shows a track count that used to be
//! recomputed with `iter().filter().count()` on every view frame. It is
//! now cached on `ComposeState` (`track_count`) and refreshed wherever
//! track membership changes. These tests pin the cache to the value the
//! old per-frame filter would have produced for each demo fixture.

use resonance_audio::types::TrackType;
use resonance_app::{demo, Resonance};

/// What the pre-cache view code computed every frame: top-level
/// (non-sub) Instrument + Vocal tracks.
fn expected_count(app: &Resonance) -> usize {
    app.test_registry()
        .tracks
        .iter()
        .filter(|t| {
            matches!(t.track_type, TrackType::Instrument | TrackType::Vocal)
                && t.sub_track.is_none()
        })
        .count()
}

#[test]
fn demo_seed_refreshes_cached_track_count() {
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);

    let expected = expected_count(&app);
    // Demo content: 4 instrument tracks + 1 vocal track (the audio
    // bounce track doesn't count).
    assert_eq!(expected, 5);
    assert_eq!(app.compose_state().track_count, expected);
}

#[test]
fn sub_tracks_are_excluded_from_cached_track_count() {
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_with_drum_subtracks(&mut app);

    let expected = expected_count(&app);
    // 4 instrument tracks; the 4 drum sub-tracks must not count.
    assert_eq!(expected, 4);
    assert_eq!(app.compose_state().track_count, expected);
}

#[test]
fn many_synth_tracks_seed_refreshes_cached_track_count() {
    let (mut app, _task) = Resonance::new();
    demo::seed_many_synth_tracks(&mut app, 7);

    assert_eq!(app.compose_state().track_count, 7);
    assert_eq!(app.compose_state().track_count, expected_count(&app));
}
