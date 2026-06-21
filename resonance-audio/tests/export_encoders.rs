//! Encoder-sink round-trip tests for the offline export pipeline (ba todo
//! #650 / doc #196). Exercises [`encode_buffer_for_test`] — the real
//! sink + resampler pipeline `run_export` drives — over a synthetic mix,
//! then decodes the output with `symphonia` (via `resonance_common`) to
//! prove every format reconstructs the signal at the right sample rate and
//! that the reported byte size matches the file on disk.
//!
//! Coverage:
//! * 32-bit-float WAV is byte-for-byte identical to a hand-written hound
//!   file (the legacy `BounceToWav` output is unchanged).
//! * 16/24-bit WAV and 16/24-bit FLAC decode back to the source within a
//!   bit-depth-appropriate tolerance, with the input frame count preserved.
//! * Requesting a different sample rate resamples the file.
//! * MP3/Opus report `EncoderUnavailable` and leave no file behind.

use std::path::{Path, PathBuf};

use resonance_audio::__test_support::encode_buffer_for_test;
use resonance_audio::types::{BitDepth, ExportFormat, ExportMetadata, FlacLevel};
use resonance_common::{decode_file, probe_audio_file, AudioFormat};

const ENGINE_SR: u32 = 48_000;
const FRAMES: usize = 12_000; // 0.25 s — long enough for FLAC blocks, fast.

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-export-enc-{}-{}",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A half-amplitude stereo test mix: 440 Hz left, 330 Hz right. Stays well
/// inside [-1, 1] so the integer paths never clip, which keeps the
/// round-trip comparison purely about quantization error.
fn test_mix() -> Vec<f32> {
    let mut buf = Vec::with_capacity(FRAMES * 2);
    for n in 0..FRAMES {
        let t = n as f32 / ENGINE_SR as f32;
        buf.push(0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin());
        buf.push(0.5 * (2.0 * std::f32::consts::PI * 330.0 * t).sin());
    }
    buf
}

/// Decode `path` back to interleaved stereo f32 at its own sample rate (so
/// no extra resampling is applied), returning the samples.
fn decode_native(path: &Path) -> Vec<f32> {
    let info = probe_audio_file(path).expect("probe");
    let (samples, _name) = decode_file(path.to_str().unwrap(), info.sample_rate).expect("decode");
    samples
}

/// Largest absolute difference over the overlapping prefix of two
/// interleaved buffers.
fn max_abs_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

#[test]
fn wav_f32_is_byte_identical_to_hound() {
    let dir = tempdir("f32-identical");
    let mix = test_mix();

    let out = dir.join("export.wav");
    let bytes = encode_buffer_for_test(
        &ExportFormat::default_wav(),
        &ExportMetadata::default(),
        ENGINE_SR,
        &mix,
        &out,
    )
    .expect("encode f32 wav");

    // Reference: write the same samples with the same spec straight through
    // hound, exactly as the legacy bounce tail did.
    let reference = dir.join("reference.wav");
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: ENGINE_SR,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(&reference, spec).unwrap();
    for &s in &mix {
        w.write_sample(s).unwrap();
    }
    w.finalize().unwrap();

    let a = std::fs::read(&out).unwrap();
    let b = std::fs::read(&reference).unwrap();
    assert_eq!(a, b, "f32 WAV export must be byte-identical to the hound tail");
    assert_eq!(bytes, a.len() as u64, "reported byte size must match the file");
}

#[test]
fn wav_integer_depths_round_trip() {
    let dir = tempdir("wav-int");
    let mix = test_mix();

    for (depth, tol) in [(BitDepth::I16, 2.0e-4), (BitDepth::I24, 2.0e-6)] {
        let out = dir.join(format!("export-{depth:?}.wav"));
        let bytes = encode_buffer_for_test(
            &ExportFormat::Wav {
                bit_depth: depth,
                sample_rate: None,
            },
            &ExportMetadata::default(),
            ENGINE_SR,
            &mix,
            &out,
        )
        .unwrap_or_else(|e| panic!("encode {depth:?}: {e}"));

        let info = probe_audio_file(&out).expect("probe");
        assert_eq!(info.sample_rate, ENGINE_SR);
        assert_eq!(info.channels, 2);
        assert_eq!(info.frames, FRAMES as u64, "{depth:?}: frame count preserved");
        assert_eq!(bytes, std::fs::metadata(&out).unwrap().len());

        let decoded = decode_native(&out);
        assert_eq!(decoded.len(), mix.len(), "{depth:?}: sample count");
        let err = max_abs_diff(&decoded, &mix);
        assert!(err < tol, "{depth:?}: round-trip error {err} exceeds {tol}");
    }
}

#[test]
fn flac_round_trips() {
    let dir = tempdir("flac");
    let mix = test_mix();

    for (depth, tol) in [(BitDepth::I16, 2.0e-4), (BitDepth::I24, 2.0e-6)] {
        let out = dir.join(format!("export-{depth:?}.flac"));
        let bytes = encode_buffer_for_test(
            &ExportFormat::Flac {
                bit_depth: depth,
                sample_rate: None,
                compression: FlacLevel::Default,
            },
            &ExportMetadata::default(),
            ENGINE_SR,
            &mix,
            &out,
        )
        .unwrap_or_else(|e| panic!("encode flac {depth:?}: {e}"));

        let info = probe_audio_file(&out).expect("probe");
        assert_eq!(info.format, AudioFormat::Flac, "{depth:?}: container is FLAC");
        assert_eq!(info.sample_rate, ENGINE_SR);
        assert_eq!(info.channels, 2);
        assert_eq!(info.frames, FRAMES as u64, "{depth:?}: frame count preserved");
        assert_eq!(bytes, std::fs::metadata(&out).unwrap().len());

        let decoded = decode_native(&out);
        assert_eq!(decoded.len(), mix.len(), "{depth:?}: sample count");
        let err = max_abs_diff(&decoded, &mix);
        assert!(err < tol, "{depth:?}: FLAC round-trip error {err} exceeds {tol}");
    }
}

#[test]
fn flac_compression_levels_are_all_lossless() {
    // Different levels trade size for speed but must decode identically.
    let dir = tempdir("flac-levels");
    let mix = test_mix();
    for level in [FlacLevel::Fast, FlacLevel::Default, FlacLevel::Max] {
        let out = dir.join(format!("export-{level:?}.flac"));
        encode_buffer_for_test(
            &ExportFormat::Flac {
                bit_depth: BitDepth::I16,
                sample_rate: None,
                compression: level,
            },
            &ExportMetadata::default(),
            ENGINE_SR,
            &mix,
            &out,
        )
        .unwrap_or_else(|e| panic!("encode flac {level:?}: {e}"));
        let decoded = decode_native(&out);
        let err = max_abs_diff(&decoded, &mix);
        assert!(err < 2.0e-4, "{level:?}: error {err}");
    }
}

#[test]
fn resample_changes_output_sample_rate() {
    let dir = tempdir("resample");
    let mix = test_mix();

    let out = dir.join("export-44k.wav");
    encode_buffer_for_test(
        &ExportFormat::Wav {
            bit_depth: BitDepth::F32,
            sample_rate: Some(44_100),
        },
        &ExportMetadata::default(),
        ENGINE_SR,
        &mix,
        &out,
    )
    .expect("encode resampled wav");

    let info = probe_audio_file(&out).expect("probe");
    assert_eq!(info.sample_rate, 44_100, "output sample rate is the requested rate");

    // Duration is preserved within a frame or two: 12000 / 48000 s worth of
    // audio becomes ~11025 frames at 44.1 kHz.
    let expected = (FRAMES as f64 * 44_100.0 / ENGINE_SR as f64).round() as i64;
    let diff = (info.frames as i64 - expected).abs();
    assert!(diff <= 4, "frame count {} ~ {expected} (diff {diff})", info.frames);
}

#[test]
fn unavailable_encoders_leave_no_file() {
    let dir = tempdir("unavailable");
    let mix = test_mix();

    for (name, format) in [
        (
            "export.mp3",
            ExportFormat::Mp3 {
                mode: resonance_audio::types::Mp3Rate::Cbr,
                bitrate_kbps: 320,
            },
        ),
        (
            "export.opus",
            ExportFormat::Opus {
                bitrate_kbps: 192,
                optimize: resonance_audio::types::OpusOptimize::Music,
            },
        ),
    ] {
        let out = dir.join(name);
        let err = encode_buffer_for_test(
            &format,
            &ExportMetadata::default(),
            ENGINE_SR,
            &mix,
            &out,
        )
        .expect_err("encoder should be unavailable");
        assert!(err.contains("not available"), "message: {err}");
        assert!(!out.exists(), "no partial file must be left behind");
    }
}
