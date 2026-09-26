//! Regression for `engine/clips.rs::handle_load_clip_from_wav` — a
//! project recorded/created under one PipeWire sample rate, opened
//! while the engine runs at another, used to mmap the WAV verbatim and
//! play it pitched and sped: the fmt-chunk sample rate was never
//! compared to the engine rate. `ClipSource::open_wav_at_rate` now
//! reads the fmt-chunk rate and, on mismatch, resamples the PCM data
//! to the engine rate at load time (off the audio thread).

use std::path::PathBuf;

use resonance_audio::transcode_to_wav;
use resonance_audio::types::ClipSource;

fn make_tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-rate-mismatch-test-{}-{}",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write one second of a 220 Hz stereo sine at `sample_rate` and
/// return the frame count.
fn write_sine_wav(path: &std::path::Path, sample_rate: u32) -> u64 {
    let total_frames = sample_rate as usize;
    let mut samples = Vec::with_capacity(total_frames * 2);
    for i in 0..total_frames {
        let t = i as f32 / sample_rate as f32;
        let s = (2.0 * std::f32::consts::PI * 220.0 * t).sin() * 0.25;
        samples.push(s);
        samples.push(s);
    }
    transcode_to_wav(path, &samples, sample_rate).expect("write test wav");
    total_frames as u64
}

/// Count sign changes on the left channel — a rate-independent proxy
/// for pitch (220 Hz over one second is ~440 zero crossings).
fn zero_crossings(frames: &[f32]) -> usize {
    frames
        .chunks_exact(2)
        .map(|f| f[0])
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
        .count()
}

/// A WAV written at 44.1 kHz loaded into a 48 kHz engine must be
/// resampled: the frame count scales by 48000/44100 and the sine still
/// spans one second of 220 Hz at the engine rate.
#[test]
fn mismatched_rate_wav_is_resampled_to_engine_rate() {
    let dir = make_tempdir("resample");
    let wav = dir.join("clip_44100.wav");
    let source_frames = write_sine_wav(&wav, 44_100);

    let engine_rate = 48_000u32;
    let source = ClipSource::open_wav_at_rate(&wav, engine_rate).expect("open wav");

    // Resampled clips can't be mmapped verbatim; they come back as an
    // in-RAM source with no mapped path.
    assert!(
        matches!(source, ClipSource::Memory(_)),
        "mismatched-rate clip must be resampled into a Memory source"
    );
    assert!(source.mapped_path().is_none());

    let expected_frames = (source_frames as f64 * engine_rate as f64 / 44_100.0) as u64;
    let got_frames = source.frame_count();
    assert!(
        got_frames.abs_diff(expected_frames) <= 2,
        "expected ~{expected_frames} frames after resample, got {got_frames}"
    );

    // The audio must still be one second of 220 Hz when played at the
    // engine rate: ~440 zero crossings, same as the source file.
    let crossings = zero_crossings(source.as_frames());
    assert!(
        (430..=450).contains(&crossings),
        "expected ~440 zero crossings after resample, got {crossings}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A WAV already at the engine rate stays memory-mapped and untouched.
#[test]
fn matching_rate_wav_stays_mapped_and_unscaled() {
    let dir = make_tempdir("match");
    let wav = dir.join("clip_48000.wav");
    let source_frames = write_sine_wav(&wav, 48_000);

    let source = ClipSource::open_wav_at_rate(&wav, 48_000).expect("open wav");
    assert!(
        matches!(source, ClipSource::Mapped { .. }),
        "matching-rate clip must remain a zero-copy Mapped source"
    );
    assert_eq!(source.frame_count(), source_frames);
    assert_eq!(source.mapped_path(), Some(wav.as_path()));

    let _ = std::fs::remove_dir_all(&dir);
}
