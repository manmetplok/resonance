//! Clip warp persistence: `ProjectClip::warp` round-trips through the
//! project file, a load re-sends it to the engine, an unwarped clip keeps
//! the old file shape, and a pre-warp project loads unwarped.

use resonance_app::project::{
    warp_algorithm_from_tag, warp_algorithm_tag, ProjectClip, ProjectFile,
};
use resonance_app::state::{ClipState, ClipWarpState};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, FadeCurve, TrackType, WarpAlgorithm, WarpMarker};

const SR: u32 = 48_000;

fn clip(id: u64, warp: ClipWarpState) -> ClipState {
    ClipState {
        id,
        track_id: 1,
        start_sample: 0,
        duration_samples: 4 * SR as u64,
        name: format!("clip {id}"),
        total_frames: 4 * SR as u64,
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
        warp,
    }
}

fn warped() -> ClipWarpState {
    ClipWarpState {
        enabled: true,
        original_bpm: Some(93.5),
        transpose_semitones: -3.0,
        algorithm: WarpAlgorithm::Tonal,
        markers: vec![
            WarpMarker {
                source_frame: 0,
                timeline_beat: 0.0,
            },
            WarpMarker {
                source_frame: 61_604,
                timeline_beat: 2.25,
            },
        ],
    }
}

fn app_with(clips: Vec<ClipState>) -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task) = Resonance::new_for_test();
    let rx = app.test_capture_engine();
    app.test_set_sample_rate(SR);
    app.test_add_track(1, TrackType::Audio);
    for c in clips {
        app.test_push_clip(c);
    }
    (app, rx)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

#[test]
fn warp_survives_save_and_load() {
    let (app, _rx) = app_with(vec![clip(7, warped()), clip(8, ClipWarpState::default())]);
    let file = app.test_build_project_file();
    let json = serde_json::to_string_pretty(&file).expect("serialize");
    let reloaded: ProjectFile = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(reloaded, file, "the file is a serde fixed point");

    let w = reloaded.clips[0].warp.as_ref().expect("the warped clip carries warp");
    assert!(w.enabled);
    assert_eq!(w.original_bpm, Some(93.5));
    assert_eq!(w.algorithm, "tonal");
    assert_eq!(w.markers.len(), 2);
    assert!(reloaded.clips[1].warp.is_none(), "an unwarped clip writes none");
    assert!(
        !json.contains("\"warp\": null"),
        "an unwarped clip omits the key entirely"
    );

    // Load it into a fresh app: the mirror comes back and the engine is
    // told, for the warped clip only.
    let (mut fresh, rx) = app_with(Vec::new());
    let _ = drain(&rx);
    fresh.test_replay_loaded_project(reloaded);
    let loaded = |id| {
        fresh
            .test_clips()
            .iter()
            .find(|c| c.id == id)
            .expect("clip loaded")
            .warp
            .clone()
    };
    assert_eq!(loaded(7), warped());
    assert_eq!(loaded(8), ClipWarpState::default());

    let sent = drain(&rx);
    let warp_cmds: Vec<_> = sent
        .iter()
        .filter(|c| {
            matches!(
                c,
                AudioCommand::SetClipWarp { .. } | AudioCommand::SetClipWarpMarkers { .. }
            )
        })
        .collect();
    assert_eq!(warp_cmds.len(), 2, "one SetClipWarp + one SetClipWarpMarkers: {warp_cmds:?}");
    assert!(warp_cmds.iter().all(|c| matches!(
        c,
        AudioCommand::SetClipWarp { clip_id: 7, .. } | AudioCommand::SetClipWarpMarkers { clip_id: 7, .. }
    )));
    // ...and after the clip's load, which the engine parks them behind.
    let load_at = sent
        .iter()
        .position(|c| matches!(c, AudioCommand::LoadClipFromWav { clip_id: 7, .. }))
        .expect("clip 7 loads");
    let warp_at = sent
        .iter()
        .position(|c| matches!(c, AudioCommand::SetClipWarp { clip_id: 7, .. }))
        .unwrap();
    assert!(load_at < warp_at);
}

#[test]
fn a_pre_warp_clip_loads_unwarped() {
    let legacy = r#"{
        "id": 7,
        "track_id": 1,
        "start_sample": 0,
        "name": "old clip",
        "total_frames": 96000,
        "trim_start_frames": 0,
        "trim_end_frames": 0,
        "audio_file": "audio/clip_7.wav",
        "gain_db": -3.0
    }"#;
    let pc: ProjectClip = serde_json::from_str(legacy).expect("legacy clip loads");
    assert!(pc.warp.is_none());
}

#[test]
fn a_hand_edited_warp_entry_is_sanitised_on_load() {
    let (app, _rx) = app_with(vec![clip(7, ClipWarpState::default())]);
    let mut file = app.test_build_project_file();
    file.clips[0].warp = serde_json::from_str(
        r#"{
            "enabled": true,
            "original_bpm": 5000.0,
            "algorithm": "granular",
            "markers": [
                {"source_frame": 48000, "timeline_beat": 2.0},
                {"source_frame": 0, "timeline_beat": 0.0}
            ]
        }"#,
    )
    .expect("partial entry parses");
    let (mut fresh, _rx) = app_with(Vec::new());
    fresh.test_replay_loaded_project(file);
    let w = fresh.test_clips()[0].warp.clone();
    assert!(w.enabled);
    assert_eq!(w.original_bpm, Some(resonance_app::state::MAX_WARP_BPM));
    assert_eq!(w.algorithm, WarpAlgorithm::default(), "an unknown tag falls back");
    assert_eq!(w.transpose_semitones, 0.0, "a missing field defaults");
    assert_eq!(w.markers[0].timeline_beat, 0.0, "markers are sorted");
}

#[test]
fn warp_algorithm_tags_round_trip() {
    for a in [WarpAlgorithm::Transient, WarpAlgorithm::Tonal] {
        assert_eq!(warp_algorithm_from_tag(warp_algorithm_tag(a)), a);
    }
}
