//! Decode coverage for `read_freeze_cache` (ba todo #577): the project
//! load path re-attaches a frozen track's cache by decoding the WAV that
//! `to_freeze_cache` wrote. A valid 32-bit float stereo file decodes into
//! a timeline-aligned [`FrozenSource`]; a missing or malformed file errors
//! (so the caller can fall back to a stale/refreeze state) rather than
//! panicking.

use std::path::PathBuf;

use resonance_audio::read_freeze_cache;
use resonance_audio::types::FrozenSource;
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

const SR: u32 = 48_000;

fn cache_ref() -> FreezeCacheRef {
    FreezeCacheRef::new("freeze_1.wav".to_string(), SR, 32, 0, FreezeCacheStatus::Frozen)
}

/// A fresh, empty temp directory unique to this test process + `tag`.
fn make_tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance_freeze_cache_read_{tag}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_wav(path: &std::path::Path, frames: &[(f32, f32)]) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: SR,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for (l, r) in frames {
        w.write_sample(*l).unwrap();
        w.write_sample(*r).unwrap();
    }
    w.finalize().unwrap();
}

#[test]
fn decodes_stereo_float_cache() {
    let dir = make_tempdir("decode");
    let path = dir.join("freeze_1.wav");
    let frames = [(0.25f32, -0.25f32), (0.5, -0.5), (0.0, 1.0)];
    write_wav(&path, &frames);

    let source: FrozenSource = read_freeze_cache(&path, cache_ref()).expect("decode cache");

    assert_eq!(source.sample_rate, SR);
    assert_eq!(source.frame_count, frames.len() as u64);
    assert_eq!(source.samples.len(), frames.len() * 2);
    // Interleaved L/R, in order.
    assert_eq!(source.samples[0], 0.25);
    assert_eq!(source.samples[1], -0.25);
    assert_eq!(source.samples[5], 1.0);
    // The cache ref travels through unchanged.
    assert_eq!(source.cache_ref.cache_filename, "freeze_1.wav");
}

#[test]
fn missing_file_errors() {
    let dir = make_tempdir("missing");
    let path = dir.join("does_not_exist.wav");
    assert!(read_freeze_cache(&path, cache_ref()).is_err());
}

#[test]
fn corrupt_file_errors() {
    let dir = make_tempdir("corrupt");
    let path = dir.join("freeze_1.wav");
    std::fs::write(&path, b"definitely not a RIFF/WAVE file").unwrap();
    assert!(read_freeze_cache(&path, cache_ref()).is_err());
}
