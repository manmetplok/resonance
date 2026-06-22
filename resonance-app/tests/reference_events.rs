//! Coverage for folding reference-track (A/B) engine events into
//! `ReferenceState`. Engine events bypass the startup gate, so these
//! drive `test_handle_engine_event` directly on a fresh app.

use resonance_app::reference::ReferenceStatus;
use resonance_app::Resonance;
use resonance_audio::types::{
    ABSource, AudioEvent, ReferenceAnalysisStage, ReferenceId,
};

fn app() -> Resonance {
    Resonance::new().0
}

fn fold(app: &mut Resonance, e: AudioEvent) {
    app.test_handle_engine_event(e);
}

#[test]
fn analysis_progress_registers_provisional_entry_from_pending_path() {
    let mut app = app();
    // Simulate a dispatched load that queued its path. The first analysis
    // event for the engine-allocated id recovers the name from that path.
    app.test_reference_push_pending("/refs/song.wav");

    fold(
        &mut app,
        AudioEvent::ReferenceAnalysisProgress {
            id: ReferenceId(3),
            stage: ReferenceAnalysisStage::MeasuringLufs,
        },
    );

    let st = app.test_reference();
    assert_eq!(st.entries.len(), 1);
    let entry = &st.entries[0];
    assert_eq!(entry.id, ReferenceId(3));
    assert_eq!(entry.name, "song");
    assert_eq!(entry.path, "/refs/song.wav");
    assert_eq!(
        entry.status,
        ReferenceStatus::Analyzing(ReferenceAnalysisStage::MeasuringLufs)
    );
    assert!(st.pending_loads.is_empty());
}

#[test]
fn loaded_upserts_entry_to_loaded() {
    let mut app = app();
    // Progress first, then the terminal load fills in measured values.
    app.test_reference_push_pending("/refs/x.wav");
    fold(
        &mut app,
        AudioEvent::ReferenceAnalysisProgress {
            id: ReferenceId(1),
            stage: ReferenceAnalysisStage::Decoding,
        },
    );
    fold(
        &mut app,
        AudioEvent::ReferenceLoaded {
            id: ReferenceId(1),
            name: "Final".to_string(),
            path: "/refs/x.wav".to_string(),
            integrated_lufs: -12.0,
            waveform_peaks: vec![(-1.0, 1.0), (-0.2, 0.3)],
        },
    );

    let st = app.test_reference();
    assert_eq!(st.entries.len(), 1, "upsert, not duplicate");
    let e = &st.entries[0];
    assert_eq!(e.status, ReferenceStatus::Loaded);
    assert_eq!(e.name, "Final");
    assert_eq!(e.integrated_lufs, -12.0);
    assert_eq!(e.waveform_peaks.len(), 2);
}

#[test]
fn loaded_without_prior_progress_registers_directly() {
    let mut app = app();
    app.test_reference_push_pending("/refs/x.wav");
    fold(
        &mut app,
        AudioEvent::ReferenceLoaded {
            id: ReferenceId(5),
            name: "Direct".to_string(),
            path: "/refs/x.wav".to_string(),
            integrated_lufs: -9.0,
            waveform_peaks: vec![],
        },
    );
    let st = app.test_reference();
    assert_eq!(st.entries.len(), 1);
    assert_eq!(st.entries[0].id, ReferenceId(5));
    assert!(st.pending_loads.is_empty(), "pending path drained");
}

#[test]
fn load_failed_sets_error_and_drains_pending() {
    let mut app = app();
    app.test_reference_push_pending("/refs/missing.wav");
    fold(
        &mut app,
        AudioEvent::ReferenceLoadFailed {
            path: "/refs/missing.wav".to_string(),
            reason: "file not found".to_string(),
        },
    );
    let st = app.test_reference();
    assert!(st.entries.is_empty());
    assert!(st.pending_loads.is_empty());
    let err = st.last_error.as_deref().unwrap();
    assert!(err.contains("missing.wav"));
    assert!(err.contains("file not found"));
}

#[test]
fn active_and_source_and_settings_fold_in() {
    let mut app = app();
    fold(&mut app, AudioEvent::ActiveReferenceChanged { id: ReferenceId(2) });
    assert_eq!(app.test_reference().active_id, Some(ReferenceId(2)));

    fold(
        &mut app,
        AudioEvent::ABSourceChanged {
            source: ABSource::Reference,
        },
    );
    assert_eq!(app.test_reference().ab_source, ABSource::Reference);

    fold(
        &mut app,
        AudioEvent::RefLoudnessMatchChanged {
            enabled: true,
            offset_db: -2.5,
        },
    );
    assert!(app.test_reference().loudness_match);
    assert_eq!(app.test_reference().offset_db, -2.5);

    fold(&mut app, AudioEvent::RefTrimChanged { db: -4.0 });
    assert_eq!(app.test_reference().trim_db, -4.0);

    fold(&mut app, AudioEvent::RefLoopToMixChanged { enabled: true });
    assert!(app.test_reference().loop_to_mix);
}

#[test]
fn markers_add_and_remove_idempotently() {
    let mut app = app();
    fold(
        &mut app,
        AudioEvent::ReferenceLoaded {
            id: ReferenceId(1),
            name: "a".to_string(),
            path: "/a.wav".to_string(),
            integrated_lufs: -14.0,
            waveform_peaks: vec![],
        },
    );
    let add = || AudioEvent::RefMarkerAdded {
        ref_id: ReferenceId(1),
        marker_id: 9,
        position_samples: 2400,
        label: "chorus".to_string(),
    };
    fold(&mut app, add());
    fold(&mut app, add()); // duplicate id — ignored
    assert_eq!(app.test_reference().entries[0].markers.len(), 1);

    fold(
        &mut app,
        AudioEvent::RefMarkerRemoved {
            ref_id: ReferenceId(1),
            marker_id: 9,
        },
    );
    assert!(app.test_reference().entries[0].markers.is_empty());
}

#[test]
fn position_changed_updates_entry_cursor() {
    let mut app = app();
    fold(
        &mut app,
        AudioEvent::ReferenceLoaded {
            id: ReferenceId(1),
            name: "a".to_string(),
            path: "/a.wav".to_string(),
            integrated_lufs: -14.0,
            waveform_peaks: vec![],
        },
    );
    fold(
        &mut app,
        AudioEvent::RefPositionChanged {
            ref_id: ReferenceId(1),
            position_samples: 48_000,
        },
    );
    assert_eq!(app.test_reference().entries[0].position_samples, 48_000);
}

#[test]
fn removed_event_drops_entry_and_active() {
    let mut app = app();
    fold(
        &mut app,
        AudioEvent::ReferenceLoaded {
            id: ReferenceId(1),
            name: "a".to_string(),
            path: "/a.wav".to_string(),
            integrated_lufs: -14.0,
            waveform_peaks: vec![],
        },
    );
    fold(&mut app, AudioEvent::ActiveReferenceChanged { id: ReferenceId(1) });
    fold(&mut app, AudioEvent::ReferenceRemoved { id: ReferenceId(1) });
    let st = app.test_reference();
    assert!(st.entries.is_empty());
    assert_eq!(st.active_id, None);
}

#[test]
fn ab_meter_snapshot_is_stored() {
    let mut app = app();
    let mix = resonance_metering::MeterSnapshot::default();
    fold(
        &mut app,
        AudioEvent::ABMeterSnapshot {
            mix,
            reference: Some(resonance_metering::MeterSnapshot::default()),
        },
    );
    let meters = app.test_reference().ab_meter.as_ref().unwrap();
    assert!(meters.reference.is_some());
}
