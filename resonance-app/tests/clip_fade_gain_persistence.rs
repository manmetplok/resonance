//! Project persistence + replay-diff round-trip for per-clip fades and
//! gain (todo #321, epic #18, arch doc #156).
//!
//! Covers the three DoD points:
//!   1. A project with fades/gain serializes and reloads identically
//!      (`ProjectClip` fields survive a `serde_json` round-trip).
//!   2. Undo/redo of a fade or gain edit restores the prior value through
//!      the fast snapshot-diff replay path (`try_diff_replay`), without a
//!      full reload.
//!   3. Older project files (saved before fades existed) still load — the
//!      new fields default to no-fade / unity gain / `EqualPower` curves.

use resonance_app::message::{ClipMessage, Message};
use resonance_app::project::{
    fade_curve_from_tag, fade_curve_tag, ProjectClip,
};
use resonance_app::state::ClipState;
use resonance_app::Resonance;
use resonance_audio::types::FadeCurve;

const SR: u32 = 48_000;

/// A 2-second audio clip with explicit fade/gain state.
fn clip(
    id: u64,
    fade_in_frames: u64,
    fade_in_curve: FadeCurve,
    fade_out_frames: u64,
    fade_out_curve: FadeCurve,
    gain_db: f32,
) -> ClipState {
    ClipState {
        id,
        track_id: 1,
        start_sample: 0,
        duration_samples: 2 * SR as u64,
        name: format!("clip {id}"),
        total_frames: 2 * SR as u64,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames,
        fade_in_curve,
        fade_out_frames,
        fade_out_curve,
        gain_db,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
    }
}

fn app_with_clip(c: ClipState) -> Resonance {
    let (mut app, _task) = Resonance::new();
    let _rx = app.test_capture_engine();
    app.test_set_sample_rate(SR);
    app.test_push_clip(c);
    app
}

fn clip_of(app: &Resonance, id: u64) -> ClipState {
    app.test_clips()
        .iter()
        .find(|c| c.id == id)
        .unwrap()
        .clone()
}

// ---------------------------------------------------------------------
// 1. Save/load round-trip
// ---------------------------------------------------------------------

#[test]
fn fade_gain_survive_serde_round_trip() {
    let app = app_with_clip(clip(
        7,
        4_800,
        FadeCurve::Linear,
        9_600,
        FadeCurve::Exp,
        -6.5,
    ));

    let file = app.test_build_project_file();
    let pc = &file.clips[0];
    // The builder mirrored ClipState fade/gain into ProjectClip, encoding
    // the curves as tags (FadeCurve has no serde derive).
    assert_eq!(pc.fade_in_frames, 4_800);
    assert_eq!(pc.fade_in_curve, "linear");
    assert_eq!(pc.fade_out_frames, 9_600);
    assert_eq!(pc.fade_out_curve, "exp");
    assert_eq!(pc.gain_db, -6.5);

    // Serialize the whole project to JSON and parse it back: the fields
    // must come back bit-identical.
    let json = serde_json::to_string_pretty(&file).expect("serialize");
    let reloaded: resonance_app::project::ProjectFile =
        serde_json::from_str(&json).expect("deserialize");
    let rc = &reloaded.clips[0];
    assert_eq!(rc.fade_in_frames, pc.fade_in_frames);
    assert_eq!(rc.fade_in_curve, pc.fade_in_curve);
    assert_eq!(rc.fade_out_frames, pc.fade_out_frames);
    assert_eq!(rc.fade_out_curve, pc.fade_out_curve);
    assert_eq!(rc.gain_db, pc.gain_db);

    // And the tags resolve back to the exact engine curves.
    assert_eq!(fade_curve_from_tag(&rc.fade_in_curve), FadeCurve::Linear);
    assert_eq!(fade_curve_from_tag(&rc.fade_out_curve), FadeCurve::Exp);
}

#[test]
fn fade_curve_tag_round_trips_every_variant() {
    for curve in [FadeCurve::Linear, FadeCurve::EqualPower, FadeCurve::Exp] {
        assert_eq!(fade_curve_from_tag(fade_curve_tag(curve)), curve);
    }
    // Unknown / hand-edited tags fall back to the default rather than
    // failing the load.
    assert_eq!(fade_curve_from_tag("nonsense"), FadeCurve::default());
}

// ---------------------------------------------------------------------
// 2. Undo/redo through the fast snapshot-diff replay path
// ---------------------------------------------------------------------

#[test]
fn undo_redo_fast_path_restores_fade_and_gain() {
    let mut app = app_with_clip(clip(
        7,
        0,
        FadeCurve::default(),
        0,
        FadeCurve::default(),
        0.0,
    ));

    // State A: the pristine, unfaded/unity clip.
    let snap_a = app.test_snapshot_for_undo();

    // Edit it: add a fade-in and lower the gain. These edits mutate the
    // ClipState mirror synchronously (the engine echo would only clamp).
    app.test_dispatch(Message::Clip(ClipMessage::SetClipFadeInMs {
        clip_id: 7,
        ms: 100.0, // 100ms @ 48k = 4_800 frames
    }));
    app.test_dispatch(Message::Clip(ClipMessage::SetClipFadeInCurve {
        clip_id: 7,
        curve: FadeCurve::Linear,
    }));
    app.test_dispatch(Message::Clip(ClipMessage::SetClipGainDb {
        clip_id: 7,
        gain_db: -6.0,
    }));

    let edited = clip_of(&app, 7);
    assert_eq!(edited.fade_in_frames, 4_800);
    assert_eq!(edited.fade_in_curve, FadeCurve::Linear);
    assert_eq!(edited.gain_db, -6.0);

    // State B: the edited clip — what redo must restore.
    let snap_b = app.test_snapshot_for_undo();

    // Undo: restore snapshot A. Same clip set / file / length, so the
    // fast diff-replay path applies the fade/gain surgically.
    app.test_begin_restore_from_snapshot(snap_a);
    let undone = clip_of(&app, 7);
    assert_eq!(undone.fade_in_frames, 0, "undo clears the fade-in");
    assert_eq!(undone.fade_in_curve, FadeCurve::EqualPower);
    assert_eq!(undone.gain_db, 0.0, "undo restores unity gain");

    // Redo: restore snapshot B.
    app.test_begin_restore_from_snapshot(snap_b);
    let redone = clip_of(&app, 7);
    assert_eq!(redone.fade_in_frames, 4_800, "redo re-applies the fade-in");
    assert_eq!(redone.fade_in_curve, FadeCurve::Linear);
    assert_eq!(redone.gain_db, -6.0, "redo re-applies the gain");
}

// ---------------------------------------------------------------------
// 3. Backward-compatible loading of pre-fade projects
// ---------------------------------------------------------------------

#[test]
fn legacy_clip_without_fade_fields_loads_with_defaults() {
    // A project clip as written by a build that predates epic #18: no
    // fade/gain fields at all (and no `asset_ref`, also serde-defaulted).
    let legacy = r#"{
        "id": 7,
        "track_id": 1,
        "start_sample": 0,
        "name": "old clip",
        "total_frames": 96000,
        "trim_start_frames": 0,
        "trim_end_frames": 0,
        "audio_file": "audio/clip_7.wav"
    }"#;

    let pc: ProjectClip = serde_json::from_str(legacy).expect("legacy clip loads");
    assert_eq!(pc.fade_in_frames, 0);
    assert_eq!(pc.fade_out_frames, 0);
    assert_eq!(pc.gain_db, 0.0);
    // Curve tags default to the engine default (EqualPower).
    assert_eq!(fade_curve_from_tag(&pc.fade_in_curve), FadeCurve::default());
    assert_eq!(fade_curve_from_tag(&pc.fade_out_curve), FadeCurve::default());
    assert_eq!(pc.asset_ref, None);
}
