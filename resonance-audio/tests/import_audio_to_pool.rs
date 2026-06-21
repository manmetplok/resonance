//! Integration coverage for the audio import-to-pool path
//! (`engine/import_pool.rs`): the pure per-file import
//! (`import_one_to_pool`) and the full ordered event lifecycle
//! (`run_pool_import`).
//!
//! The pool import decodes each source file, channel up/down-mixes and
//! resamples it to the project rate, copies the engine-format stereo WAV
//! into `{project_dir}/audio/asset_{id}.wav`, and computes waveform
//! peaks — emitting `ImportProgress` (Queued → Working → Done) plus a
//! final `AssetImported` per file, or `ImportFailed` on error. These
//! tests author mixed-rate / mixed-channel source WAVs, run a batch that
//! also includes a bad path, and assert: engine-format WAVs land under
//! `audio/`, the events arrive in the right order, and the reported
//! metadata (source channels/rate, project-rate duration) is correct.

use std::path::{Path, PathBuf};

use hound::{SampleFormat, WavSpec, WavWriter};

use resonance_audio::types::{AudioEvent, ImportStage};
use resonance_audio::{import_one_to_pool, run_pool_import, AudioFormat, ClipSource};

const PROJECT_RATE: u32 = 48_000;

fn make_tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-pool-import-{}-{}",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write an `channels`-channel f32 WAV at `sample_rate` holding `frames`
/// frames of a 220 Hz sine (same value in every channel). Returns the
/// path written.
fn write_wav(
    dir: &Path,
    name: &str,
    sample_rate: u32,
    channels: u16,
    frames: usize,
) -> String {
    let path = dir.join(name);
    let spec = WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(&path, spec).expect("create wav");
    for i in 0..frames {
        let t = i as f32 / sample_rate as f32;
        let s = (2.0 * std::f32::consts::PI * 220.0 * t).sin() * 0.5;
        for _ in 0..channels {
            writer.write_sample(s).expect("write sample");
        }
    }
    writer.finalize().expect("finalize wav");
    path.to_string_lossy().into_owned()
}

#[test]
fn import_one_resamples_upmixes_and_writes_engine_wav() {
    let dir = make_tempdir("one");
    let audio_dir = dir.join("audio");

    // A mono 44.1 kHz source: exercises both channel up-mix (1 → 2) and
    // resample (44_100 → 48_000). One second of audio.
    let src = write_wav(&dir, "mono44k.wav", 44_100, 1, 44_100);

    let outcome = import_one_to_pool(7, &src, &dir, PROJECT_RATE).expect("import ok");

    // Source metadata is reported as-is (pre-mix, pre-resample).
    assert_eq!(outcome.format, AudioFormat::Wav);
    assert_eq!(outcome.channels, 1);
    assert_eq!(outcome.source_sample_rate, 44_100);
    assert_eq!(outcome.asset_id, 7);
    assert_eq!(outcome.project_relative_path, "audio/asset_7.wav");
    assert_eq!(outcome.original_path, src);
    assert!(!outcome.peaks.is_empty());

    // The engine-format WAV exists under audio/ with the stable name.
    let written = audio_dir.join("asset_7.wav");
    assert!(written.exists(), "engine WAV must be written under audio/");

    // Reopen it: it must be stereo at the project rate, and its frame
    // count must match the reported duration (resampled length).
    let source = ClipSource::open_wav(&written).expect("reopen engine wav");
    let frames = source.frame_count();
    assert_eq!(frames, outcome.duration_frames);

    // 1 s at 44.1 kHz resampled to 48 kHz ≈ 48_000 frames (allow a small
    // tolerance for the linear resampler's endpoint handling).
    let diff = (frames as i64 - 48_000).unsigned_abs();
    assert!(
        diff <= 64,
        "resampled duration {frames} should be ~48_000 frames (diff {diff})"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_one_downmixes_and_preserves_matched_rate() {
    let dir = make_tempdir("downmix");

    // A 3-channel 48 kHz source at the project rate: no resample, but a
    // 3 → 2 channel down-mix (decode keeps the first two channels).
    let src = write_wav(&dir, "tri48k.wav", 48_000, 3, 24_000);
    let outcome = import_one_to_pool(1, &src, &dir, PROJECT_RATE).expect("import ok");

    assert_eq!(outcome.channels, 3);
    assert_eq!(outcome.source_sample_rate, 48_000);
    // Rate matches, so the half-second source stays 24_000 frames.
    assert_eq!(outcome.duration_frames, 24_000);

    let written = dir.join("audio/asset_1.wav");
    let source = ClipSource::open_wav(&written).expect("reopen");
    assert_eq!(source.frame_count(), 24_000);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_one_reports_error_for_missing_file() {
    let dir = make_tempdir("missing");
    let missing = dir.join("does-not-exist.wav");
    let err = import_one_to_pool(1, &missing.to_string_lossy(), &dir, PROJECT_RATE)
        .expect_err("missing file must error");
    assert!(!err.is_empty());
    // No stray asset file gets written on failure.
    assert!(!dir.join("audio/asset_1.wav").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn batch_emits_ordered_lifecycle_with_failures_interleaved() {
    let dir = make_tempdir("batch");

    let a = write_wav(&dir, "a.wav", 44_100, 2, 22_050); // stereo, resampled
    let bad = dir.join("missing.wav").to_string_lossy().into_owned();
    let c = write_wav(&dir, "c.wave", 48_000, 1, 48_000); // mono, .wave ext, up-mixed

    let jobs = vec![(10, a.clone()), (11, bad.clone()), (12, c.clone())];

    let mut events: Vec<AudioEvent> = Vec::new();
    run_pool_import(&jobs, &dir, PROJECT_RATE, |ev| events.push(ev));

    // 1. Every file is queued up front, in job order, before any work.
    let queued: Vec<u64> = events
        .iter()
        .take_while(|e| matches!(e, AudioEvent::ImportProgress { stage: ImportStage::Queued, .. }))
        .map(|e| match e {
            AudioEvent::ImportProgress { asset_id, .. } => *asset_id,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(queued, vec![10, 11, 12], "all files queued first, in order");

    // 2. Per-file: the events after the queued prefix must be
    //    Working(10), AssetImported(10), Done(10),
    //    Working(11), ImportFailed(11),
    //    Working(12), AssetImported(12), Done(12).
    let tail = &events[3..];
    let mut it = tail.iter();

    assert!(matches!(
        it.next(),
        Some(AudioEvent::ImportProgress { asset_id: 10, stage: ImportStage::Working, .. })
    ));
    match it.next() {
        Some(AudioEvent::AssetImported {
            asset_id: 10,
            project_relative_path,
            channels: 2,
            source_sample_rate: 44_100,
            duration_frames,
            peaks,
            ..
        }) => {
            assert_eq!(project_relative_path, "audio/asset_10.wav");
            assert!(*duration_frames > 0);
            assert!(!peaks.is_empty());
        }
        other => panic!("expected AssetImported(10), got {other:?}"),
    }
    assert!(matches!(
        it.next(),
        Some(AudioEvent::ImportProgress { asset_id: 10, stage: ImportStage::Done, .. })
    ));

    assert!(matches!(
        it.next(),
        Some(AudioEvent::ImportProgress { asset_id: 11, stage: ImportStage::Working, .. })
    ));
    match it.next() {
        Some(AudioEvent::ImportFailed { asset_id: 11, path, reason }) => {
            assert_eq!(path, &bad);
            assert!(!reason.is_empty());
        }
        other => panic!("expected ImportFailed(11), got {other:?}"),
    }

    assert!(matches!(
        it.next(),
        Some(AudioEvent::ImportProgress { asset_id: 12, stage: ImportStage::Working, .. })
    ));
    match it.next() {
        Some(AudioEvent::AssetImported {
            asset_id: 12,
            format: AudioFormat::Wav,
            channels: 1,
            source_sample_rate: 48_000,
            ..
        }) => {}
        other => panic!("expected AssetImported(12), got {other:?}"),
    }
    assert!(matches!(
        it.next(),
        Some(AudioEvent::ImportProgress { asset_id: 12, stage: ImportStage::Done, .. })
    ));
    assert!(it.next().is_none(), "no trailing events");

    // 3. The two successful imports each produced an engine-format WAV;
    //    the failed one did not.
    assert!(dir.join("audio/asset_10.wav").exists());
    assert!(!dir.join("audio/asset_11.wav").exists());
    assert!(dir.join("audio/asset_12.wav").exists());

    let _ = std::fs::remove_dir_all(&dir);
}
