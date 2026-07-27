//! Drag-to-timeline placement gesture (doc #175, epic #35, todo #605).
//!
//! Covers the transient drag machinery behind the placement visuals:
//!
//! * the pure pixel→(lane, snapped-sample, bar) resolver the timeline canvas
//!   calls on every pointer move ([`resolve_drop`]),
//! * the `DragMessage` state transitions (start / hover / cancel), and
//! * the drop, which fans out into a single undoable
//!   [`PoolMessage::ImportAndPlace`] through the normal update pipeline.
//!
//! The golden-image render of the drag state lives in
//! `tests/drag_placement_visuals.rs`.

use std::path::PathBuf;

use iced::Point;
use resonance_app::message::{DragMessage, DropTarget, Message};
use resonance_app::state::{DragPlacement, DraggedAsset, DropResolution};
use resonance_app::undo::{classify, UndoAction};
use resonance_app::view::timeline::placement::{resolve_drop, PlacementGeometry};
use resonance_app::Resonance;
use resonance_audio::__test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, TempoMap};
use resonance_common::audio_probe::{AudioFileEntry, AudioInfo};
use resonance_common::AudioFormat;

const SR: u32 = 48_000;

// --------------------------------------------------------------------
// Pure geometry: resolve_drop
// --------------------------------------------------------------------

fn geo() -> PlacementGeometry {
    PlacementGeometry {
        header_height: 60.0,
        track_height: 96.0,
        scroll_offset_y: 0.0,
        zoom: 100.0, // px per second
        sample_rate: SR,
        bpm: 120.0,
        time_sig_num: 4,
    }
}

#[test]
fn resolve_drop_targets_existing_lane_and_snaps() {
    let tm = TempoMap::default();
    let tracks = vec![1_u64, 2, 3];
    // x = 300 px → 3.0 s → 144_000 samples, already a beat boundary at
    // 120 BPM (24_000 samples/beat). y = 200 falls in lane index 1
    // (lanes start at y=60, each 96 tall: lane 1 spans 156..252).
    let res = resolve_drop(&geo(), &tm, &tracks, Point::new(300.0, 200.0));
    match res.target {
        DropTarget::ExistingTrack {
            track_id,
            start_sample,
        } => {
            assert_eq!(track_id, 2, "cursor in lane index 1 → second track");
            assert_eq!(start_sample, 144_000, "snapped to the beat under the cursor");
            assert_eq!(start_sample % 24_000, 0, "aligned to a beat boundary");
        }
        other => panic!("expected an existing-track target, got {other:?}"),
    }
    assert_eq!(res.lane_index, Some(1));
    assert!(res.bar_label.starts_with("Bar "), "bar label: {}", res.bar_label);
}

#[test]
fn resolve_drop_below_last_lane_is_new_track_zone() {
    let tm = TempoMap::default();
    let tracks = vec![1_u64, 2, 3];
    // y = 400 is below the last lane (lanes end at 60 + 3*96 = 348).
    let res = resolve_drop(&geo(), &tm, &tracks, Point::new(300.0, 400.0));
    assert!(
        matches!(res.target, DropTarget::NewTrack { start_sample } if start_sample == 144_000),
        "below the last lane resolves to the new-track zone, got {:?}",
        res.target
    );
    assert_eq!(res.lane_index, None, "new-track zone has no lane index");
}

#[test]
fn resolve_drop_with_no_tracks_is_new_track_zone() {
    let tm = TempoMap::default();
    let res = resolve_drop(&geo(), &tm, &[], Point::new(120.0, 80.0));
    assert!(matches!(res.target, DropTarget::NewTrack { .. }));
    assert_eq!(res.lane_index, None);
}

// --------------------------------------------------------------------
// DraggedAsset::from_entry — conversion note + length rescale
// --------------------------------------------------------------------

fn entry(path: &str, rate: u32, frames: u64) -> AudioFileEntry {
    AudioFileEntry {
        path: path.to_string(),
        info: AudioInfo {
            format: AudioFormat::Wav,
            channels: 2,
            sample_rate: rate,
            frames,
            duration_secs: frames as f64 / rate as f64,
        },
    }
}

#[test]
fn dragged_asset_flags_sample_rate_conversion() {
    // 44.1 kHz source into a 48 kHz project: flags the conversion and
    // rescales the frame count to project frames.
    let asset = DraggedAsset::from_entry(&entry("/imports/loop.wav", 44_100, 44_100), SR);
    assert_eq!(asset.name, "loop", "name is the file stem");
    assert_eq!(asset.source_sample_rate, 44_100);
    assert_eq!(asset.conversion.as_deref(), Some("→ 48 kHz"));
    // 1 s of 44.1k source → ~1 s of 48k project frames.
    assert_eq!(asset.duration_samples, 48_000);
}

#[test]
fn dragged_asset_no_conversion_when_rates_match() {
    let asset = DraggedAsset::from_entry(&entry("/imports/kick.wav", SR, 24_000), SR);
    assert_eq!(asset.conversion, None, "matching rates need no conversion");
    assert_eq!(asset.duration_samples, 24_000, "length unchanged");
}

// --------------------------------------------------------------------
// DragMessage state transitions
// --------------------------------------------------------------------

fn app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_sample_rate(SR);
    app.test_set_project_path(PathBuf::from("/proj/song.rproj"));
    let rx = app.test_capture_engine();
    (app, rx)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

fn sample_asset() -> DraggedAsset {
    DraggedAsset {
        path: PathBuf::from("/imports/loop.wav"),
        name: "loop".to_string(),
        source_sample_rate: SR,
        duration_samples: 96_000,
        conversion: None,
    }
}

fn existing_resolution(track_id: u64) -> DropResolution {
    DropResolution {
        target: DropTarget::ExistingTrack {
            track_id,
            start_sample: 0,
        },
        lane_index: Some(0),
        bar_label: "Bar 1.1".to_string(),
    }
}

#[test]
fn start_then_hover_tracks_the_drag() {
    let (mut app, _rx) = app();

    let _ = app.update(Message::Drag(DragMessage::Start(sample_asset())));
    let drag = app.test_drag_placement().expect("drag started");
    assert_eq!(drag.asset.name, "loop");
    assert!(drag.resolved.is_none(), "no target resolved before a move");

    let _ = app.update(Message::Drag(DragMessage::Hover {
        cursor: Point::new(240.0, 200.0),
        resolved: Some(existing_resolution(7)),
    }));
    let drag = app.test_drag_placement().expect("drag still in flight");
    assert_eq!(drag.cursor, Point::new(240.0, 200.0));
    assert!(matches!(
        drag.resolved.as_ref().map(|r| &r.target),
        Some(DropTarget::ExistingTrack { track_id: 7, .. })
    ));
}

#[test]
fn cancel_clears_the_drag_without_placing() {
    let (mut app, rx) = app();
    let _ = app.update(Message::Drag(DragMessage::Start(sample_asset())));
    let _ = app.update(Message::Drag(DragMessage::Cancel));
    assert!(app.test_drag_placement().is_none(), "cancel clears the drag");
    assert!(drain(&rx).is_empty(), "cancel issues no engine command");
}

// --------------------------------------------------------------------
// Drop dispatches the placement
// --------------------------------------------------------------------

#[test]
fn drop_on_existing_lane_dispatches_import_and_place() {
    use resonance_audio::types::TrackType;
    let (mut app, rx) = app();
    app.test_add_track(10, TrackType::Audio);

    app.test_set_drag_placement(DragPlacement {
        asset: sample_asset(),
        cursor: Point::new(240.0, 200.0),
        resolved: Some(DropResolution {
            target: DropTarget::ExistingTrack {
                track_id: 10,
                start_sample: 48_000,
            },
            lane_index: Some(0),
            bar_label: "Bar 2.1".to_string(),
        }),
    });

    let _ = app.update(Message::Drag(DragMessage::Drop));

    // The drag preview is cleared and the placement orchestration fired.
    assert!(app.test_drag_placement().is_none(), "drop clears the drag");
    let cmds = drain(&rx);
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(c, AudioCommand::ImportAudioToPool { .. }))
            .count(),
        1,
        "drop imports the dragged file"
    );
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(c, AudioCommand::AddTrack { .. }))
            .count(),
        0,
        "dropping on an existing lane spawns no track"
    );
    assert_eq!(app.test_pending_import_count(), 1, "one placement queued");

    // The asset lands on the target lane at the snapped position.
    app.test_handle_engine_event(AudioEvent::AssetImported {
        asset_id: 5,
        project_relative_path: "audio/asset_5.wav".to_string(),
        original_path: "/imports/loop.wav".to_string(),
        format: AudioFormat::Wav,
        channels: 2,
        source_sample_rate: SR,
        duration_frames: 96_000,
        peaks: vec![(-0.5, 0.5)],
    });
    assert_eq!(app.test_clips().len(), 1, "clip placed");
    let clip = &app.test_clips()[0];
    assert_eq!(clip.track_id, 10);
    assert_eq!(clip.start_sample, 48_000, "placed at the snapped drop sample");
}

#[test]
fn drop_on_new_track_zone_spawns_a_track() {
    let (mut app, rx) = app();
    assert!(app.test_registry().tracks.is_empty());

    app.test_set_drag_placement(DragPlacement {
        asset: sample_asset(),
        cursor: Point::new(240.0, 500.0),
        resolved: Some(DropResolution {
            target: DropTarget::NewTrack { start_sample: 0 },
            lane_index: None,
            bar_label: "Bar 1.1".to_string(),
        }),
    });

    let _ = app.update(Message::Drag(DragMessage::Drop));

    let cmds = drain(&rx);
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(c, AudioCommand::AddTrack { .. }))
            .count(),
        1,
        "new-track drop reserves and adds a track"
    );
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(c, AudioCommand::ImportAudioToPool { .. }))
            .count(),
        1,
    );
}

#[test]
fn drop_without_a_resolved_target_is_a_no_op() {
    let (mut app, rx) = app();
    app.test_set_drag_placement(DragPlacement::new(sample_asset()));

    let _ = app.update(Message::Drag(DragMessage::Drop));

    assert!(app.test_drag_placement().is_none(), "drop still clears the drag");
    assert!(
        drain(&rx).is_empty(),
        "an unresolved drop places nothing"
    );
    assert_eq!(app.test_pending_import_count(), 0);
}

#[test]
fn drop_records_exactly_one_undo_entry() {
    use resonance_audio::types::TrackType;
    let (mut app, _rx) = app();
    app.test_add_track(10, TrackType::Audio);

    app.test_set_drag_placement(DragPlacement {
        asset: sample_asset(),
        cursor: Point::new(240.0, 200.0),
        resolved: Some(existing_resolution(10)),
    });
    let _ = app.update(Message::Drag(DragMessage::Drop));
    app.test_handle_engine_event(AudioEvent::AssetImported {
        asset_id: 5,
        project_relative_path: "audio/asset_5.wav".to_string(),
        original_path: "/imports/loop.wav".to_string(),
        format: AudioFormat::Wav,
        channels: 2,
        source_sample_rate: SR,
        duration_frames: 96_000,
        peaks: vec![(-0.5, 0.5)],
    });

    // The drop borrows the single-action import+placement undo entry; the
    // transient Drag messages add none of their own.
    let entries = app.test_undo_history().test_undo_entries();
    assert_eq!(entries.len(), 1, "exactly one undo entry for the drop");
    assert!(
        entries[0].project.file.clips.is_empty(),
        "pre-drop snapshot has no placed clip"
    );
}

// --------------------------------------------------------------------
// Undo classification
// --------------------------------------------------------------------

#[test]
fn drag_messages_are_transient() {
    for msg in [
        Message::Drag(DragMessage::Start(sample_asset())),
        Message::Drag(DragMessage::Hover {
            cursor: Point::ORIGIN,
            resolved: None,
        }),
        Message::Drag(DragMessage::Cancel),
        Message::Drag(DragMessage::Drop),
    ] {
        assert!(
            matches!(classify(&msg), UndoAction::Skip),
            "drag previews never record undo directly"
        );
    }
}
