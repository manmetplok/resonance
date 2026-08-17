//! App-side coverage for the external-instrument playback source (doc
//! #257, todo #1100): the `TrackPlaybackSourceChanged` event mirror, the
//! auto-switch to `Recorded` when a take finishes recording on an
//! external-instrument track, the inspector toggle's engine dispatch +
//! optimistic mirror, and the undo round-trip of the explicit toggle.

use resonance_app::message::{ExternalInstrumentMessage as Eim, Message};
use resonance_app::state::TrackState;
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, AudioEvent, TrackId};
use resonance_common::PlaybackSource;

const TRACK: TrackId = 1;

fn app_with_track() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    // Undo bookkeeping only arms once a project path exists (the same
    // setup `reference_undo.rs` uses).
    app.test_set_project_path(std::path::PathBuf::from("/tmp/playback-source-undo.rsn"));
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    app
}

fn track_source(app: &Resonance, id: TrackId) -> PlaybackSource {
    app.test_registry()
        .tracks
        .iter()
        .find(|t| t.id == id)
        .expect("track exists")
        .playback_source
}

fn recording_finished_event(track_id: TrackId) -> AudioEvent {
    AudioEvent::RecordingFinished {
        clip_id: 42,
        track_id,
        start_sample: 1_000,
        duration_samples: 48_000,
        name: "take 1".into(),
        waveform_peaks: Vec::new(),
    }
}

// -- Event mirror --------------------------------------------------------

#[test]
fn playback_source_event_mirrors_into_track_state() {
    let mut app = app_with_track();
    assert_eq!(track_source(&app, TRACK), PlaybackSource::Live);

    app.test_apply_engine_event(AudioEvent::TrackPlaybackSourceChanged {
        track_id: TRACK,
        source: PlaybackSource::Recorded,
    });
    assert_eq!(track_source(&app, TRACK), PlaybackSource::Recorded);

    app.test_apply_engine_event(AudioEvent::TrackPlaybackSourceChanged {
        track_id: TRACK,
        source: PlaybackSource::Live,
    });
    assert_eq!(track_source(&app, TRACK), PlaybackSource::Live);
}

#[test]
fn playback_source_event_for_unknown_track_is_a_noop() {
    let mut app = app_with_track();
    app.test_apply_engine_event(AudioEvent::TrackPlaybackSourceChanged {
        track_id: 99,
        source: PlaybackSource::Recorded,
    });
    assert_eq!(track_source(&app, TRACK), PlaybackSource::Live);
}

// -- Auto-switch after a take --------------------------------------------

/// After a recording finishes on an external-instrument track, the mode
/// flips to `Recorded` without user action and the matching engine
/// command is dispatched (the take takes over playback).
#[test]
fn recording_finished_auto_switches_external_track_to_recorded() {
    let mut app = app_with_track();
    let _ = app.update(Message::ExternalInstrument(Eim::Enable(TRACK)));
    let cmd_rx = app.test_capture_engine();

    app.test_apply_engine_event(recording_finished_event(TRACK));

    assert_eq!(track_source(&app, TRACK), PlaybackSource::Recorded);
    // The recorded clip landed in the mirror too.
    assert!(app.test_clips().iter().any(|c| c.id == 42));
    let sent: Vec<AudioCommand> = cmd_rx.try_iter().collect();
    assert!(
        sent.iter().any(|c| matches!(
            c,
            AudioCommand::SetTrackPlaybackSource {
                track_id: TRACK,
                source: PlaybackSource::Recorded,
            }
        )),
        "auto-switch must dispatch SetTrackPlaybackSource(Recorded), got {sent:?}"
    );
}

/// A take on a plain (non-external) track changes nothing: only tracks
/// present in the `external_instruments` map auto-switch.
#[test]
fn recording_finished_on_plain_track_stays_live() {
    let mut app = app_with_track();
    let cmd_rx = app.test_capture_engine();

    app.test_apply_engine_event(recording_finished_event(TRACK));

    assert_eq!(track_source(&app, TRACK), PlaybackSource::Live);
    let sent: Vec<AudioCommand> = cmd_rx.try_iter().collect();
    assert!(
        !sent
            .iter()
            .any(|c| matches!(c, AudioCommand::SetTrackPlaybackSource { .. })),
        "no auto-switch for a plain track, got {sent:?}"
    );
}

// -- Inspector toggle ----------------------------------------------------

/// The toggle mutates the mirror optimistically and dispatches the
/// engine command, both ways.
#[test]
fn set_playback_source_round_trips_and_dispatches() {
    let mut app = app_with_track();
    let _ = app.update(Message::ExternalInstrument(Eim::Enable(TRACK)));
    let cmd_rx = app.test_capture_engine();

    let _ = app.update(Message::ExternalInstrument(Eim::SetPlaybackSource(
        TRACK,
        PlaybackSource::Recorded,
    )));
    assert_eq!(track_source(&app, TRACK), PlaybackSource::Recorded);

    let _ = app.update(Message::ExternalInstrument(Eim::SetPlaybackSource(
        TRACK,
        PlaybackSource::Live,
    )));
    assert_eq!(track_source(&app, TRACK), PlaybackSource::Live);

    let sent: Vec<AudioCommand> = cmd_rx.try_iter().collect();
    let sources: Vec<PlaybackSource> = sent
        .iter()
        .filter_map(|c| match c {
            AudioCommand::SetTrackPlaybackSource {
                track_id: TRACK,
                source,
            } => Some(*source),
            _ => None,
        })
        .collect();
    assert_eq!(
        sources,
        vec![PlaybackSource::Recorded, PlaybackSource::Live],
        "each toggle dispatches its engine command in order"
    );
}

// -- Undo classification -------------------------------------------------

/// The explicit toggle is a recorded (undoable) edit like monitor/arm:
/// undo returns the mode to `Live`, redo re-applies `Recorded`.
#[test]
fn playback_source_toggle_is_undoable() {
    let mut app = app_with_track();
    let _ = app.update(Message::ExternalInstrument(Eim::Enable(TRACK)));

    let _ = app.update(Message::ExternalInstrument(Eim::SetPlaybackSource(
        TRACK,
        PlaybackSource::Recorded,
    )));
    assert_eq!(track_source(&app, TRACK), PlaybackSource::Recorded);

    let _ = app.update(Message::Undo);
    assert_eq!(
        track_source(&app, TRACK),
        PlaybackSource::Live,
        "undo restores the previous playback source"
    );

    let _ = app.update(Message::Redo);
    assert_eq!(
        track_source(&app, TRACK),
        PlaybackSource::Recorded,
        "redo re-applies the toggle"
    );
}
