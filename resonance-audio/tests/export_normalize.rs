//! Two-pass loudness-normalization tests for the offline export pipeline
//! (ba todo #652 / doc #196). Exercises [`normalize_buffer_for_test`] — the
//! real measure → gain-trim + true-peak brick-wall limit → sink → re-measure
//! pipeline `run_export` drives on its normalized path — over a synthetic
//! mix, then decodes the output with `symphonia` and re-measures it with
//! `resonance-metering` to prove the rendered file actually lands on target.
//!
//! Coverage:
//! * An integrated-LUFS target of −14 LUFS lands within tolerance and the
//!   limiter leaves true peak under the ceiling (the headline DONE check).
//! * When the gain trim would push peaks over the ceiling, the brick-wall
//!   limiter holds true peak at/under it.
//! * A true-peak target normalizes by measured dBTP.
//! * The normalization stage preserves the exact rendered frame count
//!   (the limiter's lookahead latency is absorbed internally).

use std::path::{Path, PathBuf};

use resonance_audio::__test_support::normalize_buffer_for_test;
use resonance_audio::types::{
    BitDepth, ExportFormat, ExportMetadata, NormalizeMode, NormalizeSpec,
};
use resonance_common::{decode_file, probe_audio_file};
use resonance_metering::{LufsMeter, TruePeakMeter};

const ENGINE_SR: u32 = 48_000;
// 2 s — comfortably longer than the 400 ms integrated-LUFS gating block so
// the integrated measurement is well populated.
const FRAMES: usize = 96_000;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-export-norm-{}-{}",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A 1 kHz sine of amplitude `amp` in both channels. A pure tone has a
/// fixed peak-to-loudness ratio, so a uniform gain trim shifts its
/// integrated LUFS by exactly that gain — the property the normalizer must
/// preserve.
fn sine(amp: f32) -> Vec<f32> {
    let mut buf = Vec::with_capacity(FRAMES * 2);
    for n in 0..FRAMES {
        let t = n as f32 / ENGINE_SR as f32;
        let s = amp * (2.0 * std::f32::consts::PI * 1000.0 * t).sin();
        buf.push(s);
        buf.push(s);
    }
    buf
}

/// Decode `path` to interleaved stereo f32 at its own sample rate.
fn decode_native(path: &Path) -> (Vec<f32>, u32) {
    let info = probe_audio_file(path).expect("probe");
    let (samples, _name) = decode_file(path.to_str().unwrap(), info.sample_rate).expect("decode");
    (samples, info.sample_rate)
}

fn deinterleave(frames: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let mut l = Vec::with_capacity(frames.len() / 2);
    let mut r = Vec::with_capacity(frames.len() / 2);
    for f in frames.chunks_exact(2) {
        l.push(f[0]);
        r.push(f[1]);
    }
    (l, r)
}

/// Integrated LUFS of an interleaved stereo buffer, measured independently
/// of the export pipeline.
fn measure_lufs(frames: &[f32], sr: u32) -> f32 {
    let (l, r) = deinterleave(frames);
    let mut m = LufsMeter::new(sr as f32);
    m.push_stereo(&l, &r);
    m.integrated_lufs()
}

/// True peak (dBTP) of an interleaved stereo buffer.
fn measure_dbtp(frames: &[f32], _sr: u32) -> f32 {
    let (l, r) = deinterleave(frames);
    let mut m = TruePeakMeter::new();
    m.push_stereo(&l, &r);
    m.peak_dbtp()
}

fn wav_f32() -> ExportFormat {
    ExportFormat::Wav {
        bit_depth: BitDepth::F32,
        sample_rate: None,
    }
}

#[test]
fn integrated_lufs_target_lands_within_tolerance() {
    let dir = tempdir("lufs-14");
    let mix = sine(0.5);
    let spec = NormalizeSpec {
        enabled: true,
        mode: NormalizeMode::IntegratedLufs,
        target_db: -14.0,
        ceiling_dbtp: -1.0,
    };

    let out = dir.join("export.wav");
    let (bytes, achieved_lufs, achieved_dbtp) =
        normalize_buffer_for_test(&wav_f32(), &spec, &ExportMetadata::default(), ENGINE_SR, &mix, &out)
            .expect("normalized export");

    assert_eq!(bytes, std::fs::metadata(&out).unwrap().len());

    // Measure the rendered output independently.
    let (decoded, sr) = decode_native(&out);
    let lufs = measure_lufs(&decoded, sr);
    let dbtp = measure_dbtp(&decoded, sr);

    assert!(
        (lufs - (-14.0)).abs() < 1.0,
        "rendered integrated LUFS {lufs} should land near -14"
    );
    assert!(
        dbtp <= spec.ceiling_dbtp + 0.2,
        "true peak {dbtp} dBTP must stay under the {} dBTP ceiling",
        spec.ceiling_dbtp
    );

    // The reported achieved figures should match the independent measure.
    let achieved_lufs = achieved_lufs.expect("normalized export reports achieved LUFS");
    assert!(
        (achieved_lufs - lufs).abs() < 0.5,
        "reported achieved LUFS {achieved_lufs} ~ measured {lufs}"
    );
    assert!(
        (achieved_dbtp - dbtp).abs() < 0.5,
        "reported achieved dBTP {achieved_dbtp} ~ measured {dbtp}"
    );
}

#[test]
fn limiter_holds_true_peak_under_ceiling() {
    // A hot tone normalized to a loud target: the gain trim pushes the peak
    // above the ceiling, so the brick-wall limiter must pull it back.
    let dir = tempdir("ceiling");
    let mix = sine(0.95);
    let spec = NormalizeSpec {
        enabled: true,
        mode: NormalizeMode::IntegratedLufs,
        target_db: -1.0,
        ceiling_dbtp: -1.0,
    };

    let out = dir.join("export.wav");
    let (_, _, achieved_dbtp) =
        normalize_buffer_for_test(&wav_f32(), &spec, &ExportMetadata::default(), ENGINE_SR, &mix, &out)
            .expect("normalized export");

    let (decoded, sr) = decode_native(&out);
    let dbtp = measure_dbtp(&decoded, sr);
    assert!(
        dbtp <= spec.ceiling_dbtp + 0.2,
        "limited true peak {dbtp} dBTP must stay at/under the {} dBTP ceiling",
        spec.ceiling_dbtp
    );
    assert!(
        achieved_dbtp <= spec.ceiling_dbtp + 0.2,
        "reported achieved dBTP {achieved_dbtp} must respect the ceiling"
    );
}

#[test]
fn true_peak_mode_targets_dbtp() {
    let dir = tempdir("true-peak");
    let mix = sine(0.5); // ~-6 dBTP, so a -3 dBTP target needs +3 dB gain.
    let spec = NormalizeSpec {
        enabled: true,
        mode: NormalizeMode::TruePeak,
        target_db: -3.0,
        ceiling_dbtp: -1.0,
    };

    let out = dir.join("export.wav");
    let (_, _, achieved_dbtp) =
        normalize_buffer_for_test(&wav_f32(), &spec, &ExportMetadata::default(), ENGINE_SR, &mix, &out)
            .expect("normalized export");

    let (decoded, sr) = decode_native(&out);
    let dbtp = measure_dbtp(&decoded, sr);
    assert!(
        (dbtp - (-3.0)).abs() < 0.4,
        "true-peak-normalized output {dbtp} dBTP should land near the -3 target"
    );
    assert!(dbtp <= spec.ceiling_dbtp + 0.2, "and stay under the ceiling");
    assert!(
        (achieved_dbtp - dbtp).abs() < 0.5,
        "reported achieved dBTP {achieved_dbtp} ~ measured {dbtp}"
    );
}

#[test]
fn normalization_preserves_frame_count() {
    // The limiter's lookahead latency must be absorbed internally so the
    // encoded file has exactly as many frames as were rendered.
    let dir = tempdir("frames");
    let mix = sine(0.5);
    let spec = NormalizeSpec {
        enabled: true,
        mode: NormalizeMode::IntegratedLufs,
        target_db: -14.0,
        ceiling_dbtp: -1.0,
    };

    let out = dir.join("export.wav");
    normalize_buffer_for_test(&wav_f32(), &spec, &ExportMetadata::default(), ENGINE_SR, &mix, &out)
        .expect("normalized export");

    let info = probe_audio_file(&out).expect("probe");
    assert_eq!(
        info.frames, FRAMES as u64,
        "normalized export must preserve the rendered frame count"
    );
    assert_eq!(info.sample_rate, ENGINE_SR);
    assert_eq!(info.channels, 2);
}
