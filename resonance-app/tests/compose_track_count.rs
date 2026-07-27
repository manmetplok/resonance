//! The Compose TRACKS group header shows a track count that used to be
//! recomputed with `iter().filter().count()` on every view frame. It is
//! now cached on `ComposeState` (`track_count`) and refreshed wherever
//! track membership changes. These tests pin the cache to the value the
//! canvas's `sorted_tracks()` filter produces — i.e. top-level
//! `Instrument` tracks that are **not** drums (those go to the drumroll
//! canvas) and **not** vocal (those go to the vocal lane).

use resonance_audio::types::TrackType;
use resonance_app::state::InstrumentType;
use resonance_app::{demo, Resonance};

/// The predicate used by `ComposeTrackCanvas::sorted_tracks()` — the rows
/// actually drawn in the Compose TRACKS canvas.
fn compose_rows_count(app: &Resonance) -> usize {
    app.test_registry()
        .tracks
        .iter()
        .filter(|t| {
            matches!(t.track_type, TrackType::Instrument)
                && t.sub_track.is_none()
                && t.instrument_type != InstrumentType::Drum
        })
        .count()
}

#[test]
fn demo_seed_refreshes_cached_track_count() {
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);

    let expected = compose_rows_count(&app);
    // Demo content: 3 synth instrument tracks (Synth Bass, Synth Pad,
    // Lead Synth). The Drums track is excluded (InstrumentType::Drum),
    // the audio bounce track is excluded (TrackType::Audio), and the
    // Lead Vocal track is excluded (TrackType::Vocal — goes to the
    // vocal lane, not the Compose TRACKS canvas).
    assert_eq!(expected, 3);
    assert_eq!(app.compose_state().track_count, expected);
}

#[test]
fn sub_tracks_are_excluded_from_cached_track_count() {
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_with_drum_subtracks(&mut app);

    let expected = compose_rows_count(&app);
    // 3 melodic instrument tracks (Synth Bass, Synth Pad, Lead);
    // the Drums parent track is excluded (InstrumentType::Drum) and
    // the 4 drum sub-tracks must not count.
    assert_eq!(expected, 3);
    assert_eq!(app.compose_state().track_count, expected);
}

#[test]
fn many_synth_tracks_seed_refreshes_cached_track_count() {
    let (mut app, _task) = Resonance::new();
    demo::seed_many_synth_tracks(&mut app, 7);

    // All 7 are plain synth-instrument tracks — all appear in the canvas.
    assert_eq!(app.compose_state().track_count, 7);
    assert_eq!(app.compose_state().track_count, compose_rows_count(&app));
}

/// Regression test for ba todo #820: tracks added from melodic presets
/// (e.g. "Bass Guitar", "Solo Guitar") were classified as
/// `TrackType::Audio + InstrumentType::Synth` and silently excluded from
/// the Compose TRACKS canvas. After fixing the presets to use
/// `track_type: "instrument"`, the engine creates a `TrackType::Instrument`
/// track (via `AddInstrumentTrack`), and `sorted_tracks()` correctly
/// includes it.
///
/// This test verifies:
/// 1. A `TrackType::Instrument + InstrumentType::Synth` track (guitar preset
///    result) **appears** in the Compose TRACKS canvas rows.
/// 2. A `TrackType::Instrument + InstrumentType::Drum` track (drum preset
///    result) does **not** appear (goes to the drumroll canvas).
/// 3. A `TrackType::Vocal` track does **not** appear (goes to the vocal lane).
/// 4. The cached header count (`track_count`) equals the row count so the
///    header label stays in sync with what the canvas draws.
#[test]
fn melodic_preset_tracks_appear_in_compose_rows() {
    use resonance_app::state::TrackState;

    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);

    // Guitar preset result: track_type:"instrument" -> new_instrument() ->
    // TrackType::Instrument; apply_preset_to_track sets InstrumentType::Synth.
    let mut guitar = TrackState::new_instrument(1, 0);
    guitar.name = "Bass Guitar".to_string();
    guitar.instrument_type = InstrumentType::Synth;
    app.test_push_track(guitar);

    // Drum preset result: TrackType::Instrument + InstrumentType::Drum.
    // Must route to the drumroll canvas, NOT the TRACKS canvas.
    let mut drums = TrackState::new_instrument(2, 1);
    drums.name = "Drums".to_string();
    drums.instrument_type = InstrumentType::Drum;
    app.test_push_track(drums);

    // Vocal preset result: track_type:"vocal" -> new_vocal() ->
    // TrackType::Vocal. Must route to the vocal lane.
    let vocal = TrackState::new_vocal(3, 2);
    app.test_push_track(vocal);

    let rows = compose_rows_count(&app);
    assert_eq!(
        rows, 1,
        "only the guitar (Instrument+Synth) track should appear in the Compose TRACKS canvas; \
         drums and vocal must be excluded"
    );
    assert_eq!(
        app.compose_state().track_count,
        rows,
        "cached header count must equal the number of rows sorted_tracks() draws"
    );

    // Also verify the correct track is the one that appears.
    let visible: Vec<_> = app
        .test_registry()
        .tracks
        .iter()
        .filter(|t| {
            matches!(t.track_type, TrackType::Instrument)
                && t.sub_track.is_none()
                && t.instrument_type != InstrumentType::Drum
        })
        .collect();
    assert_eq!(visible[0].name, "Bass Guitar");
}

/// Verify that the header count and canvas rows stay in sync after mixing
/// preset types: synth instrument tracks count, drum and vocal tracks don't.
#[test]
fn track_count_excludes_drums_and_vocals() {
    use resonance_app::state::TrackState;

    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);

    // 3 melodic instrument tracks (e.g. from guitar presets).
    for i in 0..3u64 {
        let mut t = TrackState::new_instrument(10 + i, i as usize);
        t.instrument_type = InstrumentType::Synth;
        app.test_push_track(t);
    }
    // 1 drum track.
    let mut drums = TrackState::new_instrument(20, 3);
    drums.instrument_type = InstrumentType::Drum;
    app.test_push_track(drums);
    // 1 vocal track.
    app.test_push_track(TrackState::new_vocal(30, 4));

    assert_eq!(compose_rows_count(&app), 3);
    assert_eq!(app.compose_state().track_count, 3);
}
