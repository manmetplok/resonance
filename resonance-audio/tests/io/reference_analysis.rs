//! Integration coverage for the reference decode + offline LUFS analysis
//! worker (`engine/reference.rs`, todo #692).
//!
//! `run_reference_analysis` is the headless worker body: it decodes a
//! file to the engine sample rate, measures its integrated loudness,
//! builds a downsampled waveform overview, and drives the
//! `ReferenceAnalysisProgress` → `ReferenceLoaded` / `ReferenceLoadFailed`
//! event lifecycle, reporting the decoded PCM back via the internal
//! `AudioCommand::ReferenceAnalyzed`. These tests author real WAVs, run
//! the worker against them with collecting sinks, and assert the staged
//! events, the measured loudness, the bounded overview, the decode-error
//! path, and that `handle_reference_analyzed` fills the registered entry.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use hound::{SampleFormat, WavSpec, WavWriter};

use resonance_audio::types::{
    AudioCommand, AudioEvent, ReferenceAnalysisStage, ReferenceId,
};
use resonance_audio::{
    handle_reference_analyzed, register_reference, run_reference_analysis, ReferencePlayer,
    REFERENCE_OVERVIEW_PEAKS,
};

const PROJECT_RATE: u32 = 48_000;

fn make_tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-ref-analysis-{}-{}",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Author a stereo f32 WAV of a full-scale-ish 440 Hz sine at `rate`,
/// `frames` long. Returns the path written.
fn write_sine_wav(dir: &Path, name: &str, rate: u32, frames: usize, amp: f32) -> PathBuf {
    let path = dir.join(name);
    let spec = WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(&path, spec).expect("create wav");
    for i in 0..frames {
        let t = i as f32 / rate as f32;
        let s = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * amp;
        writer.write_sample(s).expect("write L");
        writer.write_sample(s).expect("write R");
    }
    writer.finalize().expect("finalize wav");
    path
}

/// Run the worker against `path`, collecting the events it emits and the
/// commands it feeds back.
fn run(path: &Path, id: ReferenceId) -> (Vec<AudioEvent>, Vec<AudioCommand>) {
    let events: RefCell<Vec<AudioEvent>> = RefCell::new(Vec::new());
    let cmds: RefCell<Vec<AudioCommand>> = RefCell::new(Vec::new());
    run_reference_analysis(
        id,
        path,
        PROJECT_RATE,
        |ev| events.borrow_mut().push(ev),
        |cmd| cmds.borrow_mut().push(cmd),
    );
    (events.into_inner(), cmds.into_inner())
}

#[test]
fn analysis_emits_staged_progress_then_loaded() {
    let dir = make_tempdir("staged");
    // One second of a healthy-level sine — long enough to measure LUFS.
    let path = write_sine_wav(&dir, "ref.wav", PROJECT_RATE, PROJECT_RATE as usize, 0.5);

    let (events, cmds) = run(&path, ReferenceId(3));

    // The four analysis stages arrive in order, ahead of the loaded event.
    let stages: Vec<ReferenceAnalysisStage> = events
        .iter()
        .filter_map(|e| match e {
            AudioEvent::ReferenceAnalysisProgress { id, stage } => {
                assert_eq!(*id, ReferenceId(3));
                Some(*stage)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        stages,
        vec![
            ReferenceAnalysisStage::Decoding,
            ReferenceAnalysisStage::MeasuringLufs,
            ReferenceAnalysisStage::BuildingPeaks,
            ReferenceAnalysisStage::ComputingOffset,
        ]
    );

    // The final event is ReferenceLoaded with a real measurement.
    match events.last().expect("at least one event") {
        AudioEvent::ReferenceLoaded {
            id,
            name,
            path: ev_path,
            integrated_lufs,
            waveform_peaks,
            length_samples,
        } => {
            assert_eq!(*id, ReferenceId(3));
            assert_eq!(name, "ref");
            assert_eq!(ev_path, &path.to_string_lossy().into_owned());
            // A −6 dBFS sine sits well within a sane loudness range.
            assert!(
                integrated_lufs.is_finite() && *integrated_lufs < 0.0 && *integrated_lufs > -40.0,
                "implausible LUFS: {integrated_lufs}"
            );
            assert!(!waveform_peaks.is_empty());
            assert!(waveform_peaks.len() <= REFERENCE_OVERVIEW_PEAKS);
            // The reported length is the decoded frame count (non-empty file).
            assert!(*length_samples > 0, "reference length should be non-zero");
        }
        other => panic!("expected ReferenceLoaded last, got {other:?}"),
    }

    // The decoded PCM is reported back to the engine exactly once.
    assert_eq!(cmds.len(), 1);
    match &cmds[0] {
        AudioCommand::ReferenceAnalyzed {
            id,
            pcm,
            integrated_lufs,
        } => {
            assert_eq!(*id, ReferenceId(3));
            assert!(!pcm.is_empty());
            // Stereo interleaved at the project rate ≈ 1 s.
            assert_eq!(pcm.len(), PROJECT_RATE as usize * 2);
            assert!(integrated_lufs.is_finite());
        }
        other => panic!("expected ReferenceAnalyzed, got {other:?}"),
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn overview_peaks_are_bounded_for_long_input() {
    let dir = make_tempdir("bounded");
    // ~5 s — far more than REFERENCE_OVERVIEW_PEAKS frame-buckets worth,
    // so the overview must decimate rather than scale with duration.
    let frames = PROJECT_RATE as usize * 5;
    let path = write_sine_wav(&dir, "long.wav", PROJECT_RATE, frames, 0.25);

    let (events, _cmds) = run(&path, ReferenceId(1));
    match events.last().unwrap() {
        AudioEvent::ReferenceLoaded { waveform_peaks, .. } => {
            assert!(!waveform_peaks.is_empty());
            assert!(
                waveform_peaks.len() <= REFERENCE_OVERVIEW_PEAKS,
                "overview not bounded: {}",
                waveform_peaks.len()
            );
        }
        other => panic!("expected ReferenceLoaded, got {other:?}"),
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn missing_file_emits_load_failed_and_no_feedback() {
    let (events, cmds) = run(Path::new("/no/such/reference.wav"), ReferenceId(7));

    // First (and only meaningful) stage is Decoding, then failure.
    assert!(matches!(
        events.first(),
        Some(AudioEvent::ReferenceAnalysisProgress {
            stage: ReferenceAnalysisStage::Decoding,
            ..
        })
    ));
    match events.last().unwrap() {
        AudioEvent::ReferenceLoadFailed { path, reason } => {
            assert_eq!(path, "/no/such/reference.wav");
            assert!(!reason.is_empty());
        }
        other => panic!("expected ReferenceLoadFailed, got {other:?}"),
    }
    // No ReferenceLoaded, no PCM reported back.
    assert!(!events
        .iter()
        .any(|e| matches!(e, AudioEvent::ReferenceLoaded { .. })));
    assert!(cmds.is_empty());
}

#[test]
fn reference_analyzed_fills_registered_entry() {
    let dir = make_tempdir("fill");
    let path = write_sine_wav(&dir, "fill.wav", PROJECT_RATE, PROJECT_RATE as usize, 0.5);

    // The engine registers the entry up front (unanalysed)...
    let mut player = ReferencePlayer::new();
    let id = register_reference(&mut player, None, path.clone());
    assert_eq!(player.entry_has_pcm(id), Some(false));
    assert_eq!(
        player.entry_integrated_lufs(id),
        Some(f32::NEG_INFINITY),
        "fresh entry should be unmeasured"
    );

    // ...the worker decodes + measures and feeds the result back...
    let (_events, cmds) = run(&path, id);
    let analyzed = cmds
        .into_iter()
        .find(|c| matches!(c, AudioCommand::ReferenceAnalyzed { .. }))
        .expect("worker should report ReferenceAnalyzed");

    // ...which the engine stores into the entry.
    if let AudioCommand::ReferenceAnalyzed {
        id: aid,
        pcm,
        integrated_lufs,
    } = analyzed
    {
        handle_reference_analyzed(&mut player, aid, pcm, integrated_lufs);
    }
    assert_eq!(player.entry_has_pcm(id), Some(true));
    assert!(player.entry_integrated_lufs(id).unwrap().is_finite());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn reference_analyzed_for_unknown_id_is_a_noop() {
    let mut player = ReferencePlayer::new();
    // No entry registered → storing analysis results is silently dropped.
    handle_reference_analyzed(&mut player, ReferenceId(99), std::sync::Arc::new(vec![0.0; 4]), -14.0);
    assert_eq!(player.entry_has_pcm(ReferenceId(99)), None);
}
