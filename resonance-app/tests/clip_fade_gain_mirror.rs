//! Engine→app mirroring of per-clip fade and gain (todo #316, arch doc
//! #156). The engine owns the clamped fade/gain values; these tests prove
//! that receiving `AudioEvent::ClipFadeChanged` / `ClipGainChanged` folds
//! those values into the matching GUI-side `ClipState` and leaves
//! everything else (and unrelated clips) untouched.

use resonance_app::state::ClipState;
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, FadeCurve};

/// A bare audio clip with default (no-fade / unity) fade+gain state.
fn clip(id: u64, track_id: u64) -> ClipState {
    ClipState {
        id,
        track_id,
        start_sample: 1_000,
        duration_samples: 48_000,
        name: format!("clip {id}"),
        total_frames: 48_000,
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
    }
}

#[test]
fn fade_changed_mirrors_lengths_and_curves() {
    let mut app = Resonance::new().0;
    app.test_push_clip(clip(7, 1));

    app.test_apply_engine_event(AudioEvent::ClipFadeChanged {
        clip_id: 7,
        fade_in_frames: 4_800,
        fade_in_curve: FadeCurve::Linear,
        fade_out_frames: 9_600,
        fade_out_curve: FadeCurve::Exp,
    });

    let c = &app.test_clips()[0];
    assert_eq!(c.fade_in_frames, 4_800);
    assert_eq!(c.fade_in_curve, FadeCurve::Linear);
    assert_eq!(c.fade_out_frames, 9_600);
    assert_eq!(c.fade_out_curve, FadeCurve::Exp);
    // Unrelated geometry is untouched.
    assert_eq!(c.start_sample, 1_000);
    assert_eq!(c.duration_samples, 48_000);
    assert_eq!(c.gain_db, 0.0);
}

#[test]
fn gain_changed_mirrors_db() {
    let mut app = Resonance::new().0;
    app.test_push_clip(clip(7, 1));

    app.test_apply_engine_event(AudioEvent::ClipGainChanged {
        clip_id: 7,
        gain_db: -6.0,
    });

    let c = &app.test_clips()[0];
    assert_eq!(c.gain_db, -6.0);
    // Fade state is untouched by a gain event.
    assert_eq!(c.fade_in_frames, 0);
    assert_eq!(c.fade_out_frames, 0);
}

#[test]
fn events_only_touch_the_matching_clip() {
    let mut app = Resonance::new().0;
    app.test_push_clip(clip(7, 1));
    app.test_push_clip(clip(8, 1));

    app.test_apply_engine_event(AudioEvent::ClipFadeChanged {
        clip_id: 8,
        fade_in_frames: 2_400,
        fade_in_curve: FadeCurve::EqualPower,
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::EqualPower,
    });
    app.test_apply_engine_event(AudioEvent::ClipGainChanged {
        clip_id: 8,
        gain_db: 3.0,
    });

    let clips = app.test_clips();
    let c7 = clips.iter().find(|c| c.id == 7).unwrap();
    let c8 = clips.iter().find(|c| c.id == 8).unwrap();
    // Clip 7 stays at defaults.
    assert_eq!(c7.fade_in_frames, 0);
    assert_eq!(c7.gain_db, 0.0);
    // Clip 8 took both changes.
    assert_eq!(c8.fade_in_frames, 2_400);
    assert_eq!(c8.gain_db, 3.0);
}

#[test]
fn unknown_clip_id_is_a_no_op() {
    let mut app = Resonance::new().0;
    app.test_push_clip(clip(7, 1));

    // No clip with id 99 — must not panic or mutate the existing clip.
    app.test_apply_engine_event(AudioEvent::ClipFadeChanged {
        clip_id: 99,
        fade_in_frames: 1_000,
        fade_in_curve: FadeCurve::Linear,
        fade_out_frames: 1_000,
        fade_out_curve: FadeCurve::Linear,
    });
    app.test_apply_engine_event(AudioEvent::ClipGainChanged {
        clip_id: 99,
        gain_db: -12.0,
    });

    let c = &app.test_clips()[0];
    assert_eq!(c.id, 7);
    assert_eq!(c.fade_in_frames, 0);
    assert_eq!(c.gain_db, 0.0);
}
