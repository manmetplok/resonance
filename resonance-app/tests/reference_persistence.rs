//! Persistence coverage for the reference-track (A/B) block: serialize
//! the GUI mirror into the on-disk `ProjectFile`, restore it back, and
//! prove the durable facts (entries, markers, cached loudness, panel
//! settings) round-trip — while a now-missing file degrades gracefully
//! and a legacy project without the field loads cleanly.
//!
//! The restore path re-issues `LoadReferenceTrack`; the engine
//! `AudioCommand` side effect can't be observed from outside the crate,
//! so these assert the GUI-state outcome.

use std::path::PathBuf;

use resonance_app::message::Message;
use resonance_app::project::{
    ProjectFile, ProjectReference, ProjectReferenceMarker, ProjectReferenceSettings,
};
use resonance_app::reference::{ReferenceMessage, ReferenceStatus};
use resonance_app::Resonance;
use resonance_audio::types::{ABSource, ReferenceId};

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(PathBuf::from("/tmp/reference-persist-test.rsn"));
    app
}

fn send(app: &mut Resonance, m: ReferenceMessage) {
    let _ = app.update(Message::Reference(m));
}

fn fold_loaded(app: &mut Resonance, id: u32, name: &str, path: &str, lufs: f32) {
    app.test_handle_engine_event(resonance_audio::types::AudioEvent::ReferenceLoaded {
        id: ReferenceId(id),
        name: name.to_string(),
        path: path.to_string(),
        integrated_lufs: lufs,
        waveform_peaks: vec![(-0.5, 0.5)],
        length_samples: 480_000,
    });
}

fn fold_marker(app: &mut Resonance, ref_id: u32, marker_id: u32, pos: u64, label: &str) {
    app.test_handle_engine_event(resonance_audio::types::AudioEvent::RefMarkerAdded {
        ref_id: ReferenceId(ref_id),
        marker_id,
        position_samples: pos,
        label: label.to_string(),
    });
}

/// Create a real, empty temp file and return its absolute path string, so
/// the restore path's file-existence check takes the "present" branch.
fn touch_temp(tag: &str) -> String {
    let mut p = std::env::temp_dir();
    p.push(format!("resonance-ref-persist-{tag}-{}.wav", std::process::id()));
    std::fs::write(&p, b"").expect("write temp ref file");
    p.to_string_lossy().into_owned()
}

#[test]
fn build_project_file_captures_entries_and_settings() {
    let mut app = app();
    fold_loaded(&mut app, 1, "kick-ref", "/refs/kick-ref.wav", -9.0);
    fold_loaded(&mut app, 2, "vox-ref", "/refs/vox-ref.wav", -12.5);
    fold_marker(&mut app, 1, 7, 44_100, "drop");
    send(&mut app, ReferenceMessage::SetActive(ReferenceId(2)));
    send(&mut app, ReferenceMessage::ToggleLoudnessMatch);
    send(&mut app, ReferenceMessage::TrimChanged(-3.0));
    send(&mut app, ReferenceMessage::ToggleLoopToMix);
    send(&mut app, ReferenceMessage::ToggleAbSource); // -> Reference

    let file = app.test_build_project_file();

    assert_eq!(file.references.len(), 2);
    assert_eq!(file.references[0].name, "kick-ref");
    assert_eq!(file.references[0].path, "/refs/kick-ref.wav");
    assert_eq!(file.references[0].integrated_lufs, -9.0);
    assert_eq!(file.references[0].markers.len(), 1);
    assert_eq!(file.references[0].markers[0].id, 7);
    assert_eq!(file.references[0].markers[0].position_samples, 44_100);
    assert_eq!(file.references[0].markers[0].label, "drop");
    assert!(file.references[1].markers.is_empty());

    let s = &file.reference_settings;
    assert!(s.monitor_only, "persisted block must be flagged monitor-only");
    assert_eq!(s.active, Some(1), "active addressed by index (entry 2 is idx 1)");
    assert!(s.ab_source_is_reference);
    assert!(s.loudness_match);
    assert_eq!(s.trim_db, -3.0);
    assert!(s.loop_to_mix);
}

#[test]
fn restore_round_trips_entries_markers_and_settings() {
    let path_a = touch_temp("rt-a");
    let path_b = touch_temp("rt-b");

    let file = ProjectFile {
        references: vec![
            ProjectReference {
                path: path_a.clone(),
                name: "kick-ref".into(),
                integrated_lufs: -9.0,
                markers: vec![ProjectReferenceMarker {
                    id: 7,
                    position_samples: 44_100,
                    label: "drop".into(),
                }],
            },
            ProjectReference {
                path: path_b.clone(),
                name: "vox-ref".into(),
                integrated_lufs: -12.5,
                markers: vec![],
            },
        ],
        reference_settings: ProjectReferenceSettings {
            monitor_only: true,
            active: Some(1),
            ab_source_is_reference: true,
            loudness_match: true,
            trim_db: -3.0,
            loop_to_mix: true,
        },
        ..ProjectFile::default()
    };

    let mut app = app();
    app.test_restore_references(&file);

    let st = app.test_reference();
    assert_eq!(st.entries.len(), 2);
    // Present files are re-decoding; ids are reallocated 1..=K in order.
    assert_eq!(st.entries[0].id, ReferenceId(1));
    assert_eq!(st.entries[1].id, ReferenceId(2));
    assert_eq!(st.entries[0].name, "kick-ref");
    assert_eq!(st.entries[0].path, path_a);
    assert_eq!(st.entries[0].integrated_lufs, -9.0, "cached loudness preserved");
    assert!(matches!(st.entries[0].status, ReferenceStatus::Analyzing(_)));
    assert_eq!(st.entries[0].markers.len(), 1);
    assert_eq!(st.entries[0].markers[0].label, "drop");

    assert_eq!(st.active_id, Some(ReferenceId(2)), "active index maps to entry id");
    assert_eq!(st.ab_source, ABSource::Reference);
    assert!(st.loudness_match);
    assert_eq!(st.trim_db, -3.0);
    assert!(st.loop_to_mix);

    // The re-decode echoes `ReferenceLoaded`; the entry flips to Loaded and
    // gains its waveform while the restored markers survive the fold.
    fold_loaded(&mut app, 1, "kick-ref", &path_a, -9.0);
    let st = app.test_reference();
    assert_eq!(st.entries[0].status, ReferenceStatus::Loaded);
    assert!(!st.entries[0].waveform_peaks.is_empty());
    assert_eq!(st.entries[0].markers.len(), 1, "markers survive re-decode");

    let _ = std::fs::remove_file(&path_a);
    let _ = std::fs::remove_file(&path_b);
}

#[test]
fn restore_marks_missing_file_without_crashing() {
    let file = ProjectFile {
        references: vec![ProjectReference {
            path: "/no/such/reference-file-xyz.wav".into(),
            name: "ghost-ref".into(),
            integrated_lufs: -10.0,
            markers: vec![ProjectReferenceMarker {
                id: 3,
                position_samples: 1_000,
                label: "verse".into(),
            }],
        }],
        reference_settings: ProjectReferenceSettings {
            active: Some(0),
            ..ProjectReferenceSettings::default()
        },
        ..ProjectFile::default()
    };

    let mut app = app();
    app.test_restore_references(&file);

    let st = app.test_reference();
    assert_eq!(st.entries.len(), 1);
    let e = &st.entries[0];
    assert_eq!(e.status, ReferenceStatus::Missing, "missing file -> Missing badge");
    assert_eq!(e.name, "ghost-ref", "name preserved");
    assert_eq!(e.path, "/no/such/reference-file-xyz.wav", "path preserved");
    assert_eq!(e.markers.len(), 1, "markers preserved on a missing reference");
    assert!(st.last_error.is_some(), "an inline error is surfaced");
    // The active selection still resolves to the (missing) entry's id; it
    // just isn't engaged on the engine.
    assert_eq!(st.active_id, Some(e.id));
}

#[test]
fn restore_replaces_prior_project_references() {
    let mut app = app();
    // Pretend a previous project's reference is already loaded.
    fold_loaded(&mut app, 1, "old-ref", "/refs/old-ref.wav", -8.0);
    assert_eq!(app.test_reference().entries.len(), 1);

    // Loading a project with no references must wipe the stale one.
    app.test_restore_references(&ProjectFile::default());
    assert!(app.test_reference().entries.is_empty());
    assert!(app.test_reference().active_id.is_none());
}

#[test]
fn legacy_project_without_reference_field_loads_with_defaults() {
    // A project authored before the reference block existed has no
    // `references` / `reference_settings` keys. `#[serde(default)]` must
    // fill them with an empty, mix-monitoring state.
    let mut json = serde_json::to_value(ProjectFile::default()).unwrap();
    let obj = json.as_object_mut().unwrap();
    obj.remove("references");
    obj.remove("reference_settings");

    let file: ProjectFile = serde_json::from_value(json).expect("legacy project deserializes");
    assert!(file.references.is_empty());
    assert!(
        file.reference_settings.monitor_only,
        "default settings still flag monitor-only"
    );
    assert!(file.reference_settings.active.is_none());
    assert!(!file.reference_settings.ab_source_is_reference);
    assert!(!file.reference_settings.loudness_match);

    // And restoring that legacy file is a clean no-op on reference state.
    let mut app = app();
    app.test_restore_references(&file);
    assert!(app.test_reference().entries.is_empty());
}
