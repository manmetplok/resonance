//! A `ClipImported` from a replaced project is dropped (code review
//! UPD-09). An import queued before a project load or slow-path undo used
//! to land in the new project: it overwrote the waveform and length of the
//! new project's clip with the same id, or added a phantom clip on a track
//! the project doesn't have.

use resonance_app::state::{ClipState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, FadeCurve, TrackType};

const TRACK: u64 = 1;
const CLIP: u64 = 3;
const FRAMES: u64 = 48_000;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_push_clip(ClipState {
        id: CLIP,
        track_id: TRACK,
        start_sample: 0,
        duration_samples: FRAMES,
        name: "loaded".into(),
        total_frames: FRAMES,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
    });
    app
}

fn imported(clip_id: u64, track_id: u64) -> AudioEvent {
    AudioEvent::ClipImported {
        clip_id,
        track_id,
        start_sample: 0,
        duration_samples: 10 * FRAMES,
        name: "stale".into(),
        waveform_peaks: vec![(0.0, 1.0)],
    }
}

#[test]
fn a_stale_import_neither_overwrites_a_clip_nor_adds_a_phantom() {
    let mut app = app();

    app.test_apply_engine_event(imported(CLIP, 99));
    app.test_apply_engine_event(imported(12, 99));

    let clip = app.test_clips().iter().find(|c| c.id == CLIP).unwrap();
    assert_eq!(clip.total_frames, FRAMES, "clip 3 keeps its own length");
    assert!(clip.waveform_peaks.is_empty(), "and its own waveform");
    assert!(
        !app.test_clips().iter().any(|c| c.track_id == 99),
        "no clip on a track this project doesn't have"
    );
}

#[test]
fn a_current_import_still_lands() {
    let mut app = app();
    app.test_apply_engine_event(imported(12, TRACK));
    assert!(app.test_clips().iter().any(|c| c.id == 12 && c.track_id == TRACK));
}
