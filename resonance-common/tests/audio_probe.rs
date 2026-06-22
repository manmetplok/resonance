use std::path::{Path, PathBuf};

use resonance_common::{
    probe_audio_file, scan_audio_folder, waveform_thumbnail, AudioFormat,
};

/// Build a 16-bit PCM WAV file in memory. `interleaved` holds samples in
/// channel-interleaved order; its length must be a multiple of `channels`.
fn build_wav_16(channels: u16, sr: u32, interleaved: &[i16]) -> Vec<u8> {
    let block_align = channels * 2;
    let byte_rate = sr * block_align as u32;
    let data_bytes = (interleaved.len() * 2) as u32;
    let riff_size = 36 + data_bytes;

    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for s in interleaved {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// A unique temp directory for one test, wiped first so reruns start clean.
fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("resonance_test_audio_probe_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_file(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn from_extension_classifies_known_formats() {
    assert_eq!(AudioFormat::from_extension("wav"), AudioFormat::Wav);
    assert_eq!(AudioFormat::from_extension("WAV"), AudioFormat::Wav);
    assert_eq!(AudioFormat::from_extension("wave"), AudioFormat::Wav);
    assert_eq!(AudioFormat::from_extension("flac"), AudioFormat::Flac);
    assert_eq!(AudioFormat::from_extension("mp3"), AudioFormat::Mp3);
    assert_eq!(AudioFormat::from_extension("ogg"), AudioFormat::Ogg);
    assert_eq!(AudioFormat::from_extension("oga"), AudioFormat::Ogg);
    assert_eq!(AudioFormat::from_extension("aac"), AudioFormat::Aac);
    assert_eq!(AudioFormat::from_extension("m4a"), AudioFormat::Mp4);
    assert_eq!(AudioFormat::from_extension("mp4"), AudioFormat::Mp4);
    assert_eq!(AudioFormat::from_extension("txt"), AudioFormat::Other);
    assert_eq!(AudioFormat::from_extension(""), AudioFormat::Other);

    // Every scanned extension maps to a concrete (non-Other) format.
    for ext in AudioFormat::SCANNED_EXTENSIONS {
        assert_ne!(
            AudioFormat::from_extension(ext),
            AudioFormat::Other,
            "scanned extension {ext} should classify"
        );
    }
}

#[test]
fn probe_reads_mono_metadata() {
    let dir = fresh_dir("mono");
    // 44_100 mono frames = exactly 1 second at 44.1 kHz.
    let samples: Vec<i16> = (0..44_100).map(|i| (i % 100) as i16).collect();
    let wav = build_wav_16(1, 44_100, &samples);
    let path = dir.join("mono.wav");
    write_file(&path, &wav);

    let info = probe_audio_file(&path).expect("probe");
    assert_eq!(info.format, AudioFormat::Wav);
    assert_eq!(info.channels, 1);
    assert_eq!(info.sample_rate, 44_100);
    assert_eq!(info.frames, 44_100);
    assert!((info.duration_secs - 1.0).abs() < 1e-6, "duration {}", info.duration_secs);
}

#[test]
fn probe_reads_stereo_metadata() {
    let dir = fresh_dir("stereo");
    // 24_000 stereo frames at 48 kHz = 0.5 s. Interleaved => 48_000 samples.
    let interleaved: Vec<i16> = (0..24_000)
        .flat_map(|i| [(i % 50) as i16, -((i % 50) as i16)])
        .collect();
    let wav = build_wav_16(2, 48_000, &interleaved);
    let path = dir.join("stereo.wav");
    write_file(&path, &wav);

    let info = probe_audio_file(&path).expect("probe");
    assert_eq!(info.channels, 2);
    assert_eq!(info.sample_rate, 48_000);
    assert_eq!(info.frames, 24_000);
    assert!((info.duration_secs - 0.5).abs() < 1e-6, "duration {}", info.duration_secs);
}

#[test]
fn probe_rejects_non_audio_file() {
    let dir = fresh_dir("garbage");
    let path = dir.join("not_audio.wav");
    write_file(&path, b"this is not a wav file at all");
    assert!(probe_audio_file(&path).is_err());
}

#[test]
fn thumbnail_has_requested_bucket_count() {
    let dir = fresh_dir("thumb_buckets");
    let interleaved: Vec<i16> = (0..10_000).map(|i| ((i % 200) as i16) - 100).collect();
    let wav = build_wav_16(1, 48_000, &interleaved);
    let path = dir.join("ramp.wav");
    write_file(&path, &wav);

    let thumb = waveform_thumbnail(&path, 64).expect("thumbnail");
    assert_eq!(thumb.len(), 64);
    assert_eq!(thumb.min.len(), 64);
    assert_eq!(thumb.max.len(), 64);
    assert_eq!(thumb.channels, 1);
    assert_eq!(thumb.sample_rate, 48_000);
    assert_eq!(thumb.frames, 10_000);
    // min never exceeds max within a bucket.
    for (mn, mx) in thumb.min.iter().zip(thumb.max.iter()) {
        assert!(mn <= mx, "min {mn} > max {mx}");
    }
}

#[test]
fn thumbnail_locates_loud_region() {
    let dir = fresh_dir("thumb_loud");
    // Silent first half, full-scale tone in the second half. The loud
    // buckets should reach near +/-1.0 while the quiet ones stay near 0.
    let total = 8_000usize;
    let interleaved: Vec<i16> = (0..total)
        .map(|i| if i >= total / 2 { 32_000 } else { 0 })
        .collect();
    let wav = build_wav_16(1, 48_000, &interleaved);
    let path = dir.join("half.wav");
    write_file(&path, &wav);

    let thumb = waveform_thumbnail(&path, 8).expect("thumbnail");
    // First bucket sits in the silent region, last in the loud region.
    assert!(thumb.max[0].abs() < 0.01, "first bucket should be quiet: {}", thumb.max[0]);
    assert!(
        thumb.max[7] > 0.9,
        "last bucket should be near full-scale: {}",
        thumb.max[7]
    );
}

#[test]
fn thumbnail_sums_channels_to_mono() {
    let dir = fresh_dir("thumb_mono_sum");
    // L = +full, R = -full on every frame => channel average ~0.
    let interleaved: Vec<i16> = (0..4_000).flat_map(|_| [32_000i16, -32_000]).collect();
    let wav = build_wav_16(2, 48_000, &interleaved);
    let path = dir.join("antiphase.wav");
    write_file(&path, &wav);

    let thumb = waveform_thumbnail(&path, 16).expect("thumbnail");
    for (mn, mx) in thumb.min.iter().zip(thumb.max.iter()) {
        assert!(mn.abs() < 0.01 && mx.abs() < 0.01, "antiphase should cancel: {mn}/{mx}");
    }
}

#[test]
fn thumbnail_handles_fewer_frames_than_buckets() {
    let dir = fresh_dir("thumb_tiny");
    // Three frames, 8 buckets requested — must not panic and must return
    // exactly 8 buckets.
    let wav = build_wav_16(1, 48_000, &[10_000, -10_000, 20_000]);
    let path = dir.join("tiny.wav");
    write_file(&path, &wav);

    let thumb = waveform_thumbnail(&path, 8).expect("thumbnail");
    assert_eq!(thumb.len(), 8);
    assert_eq!(thumb.frames, 3);
}

#[test]
fn thumbnail_clamps_zero_buckets_to_one() {
    let dir = fresh_dir("thumb_zero");
    let wav = build_wav_16(1, 48_000, &[1_000, 2_000, 3_000]);
    let path = dir.join("z.wav");
    write_file(&path, &wav);

    let thumb = waveform_thumbnail(&path, 0).expect("thumbnail");
    assert_eq!(thumb.len(), 1);
}

#[test]
fn scan_folder_lists_audio_with_metadata_sorted() {
    let dir = fresh_dir("scan");
    // Two audio files plus a non-audio file that must be ignored.
    write_file(
        &dir.join("b_stereo.wav"),
        &build_wav_16(2, 48_000, &vec![0i16; 48_000 * 2]),
    );
    write_file(
        &dir.join("a_mono.wav"),
        &build_wav_16(1, 22_050, &vec![0i16; 22_050]),
    );
    write_file(&dir.join("notes.txt"), b"ignore me");

    let entries = scan_audio_folder(&dir);
    assert_eq!(entries.len(), 2, "txt file must be skipped");
    // Sorted by path: a_mono before b_stereo.
    assert!(entries[0].path.ends_with("a_mono.wav"));
    assert!(entries[1].path.ends_with("b_stereo.wav"));

    assert_eq!(entries[0].info.channels, 1);
    assert_eq!(entries[0].info.sample_rate, 22_050);
    assert_eq!(entries[0].info.frames, 22_050);

    assert_eq!(entries[1].info.channels, 2);
    assert_eq!(entries[1].info.sample_rate, 48_000);
    assert_eq!(entries[1].info.frames, 48_000);
    assert!((entries[1].info.duration_secs - 1.0).abs() < 1e-6);
}

#[test]
fn scan_skips_corrupt_files() {
    let dir = fresh_dir("scan_corrupt");
    write_file(&dir.join("good.wav"), &build_wav_16(1, 48_000, &[0i16; 480]));
    write_file(&dir.join("bad.wav"), b"RIFFnope");

    let entries = scan_audio_folder(&dir);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].path.ends_with("good.wav"));
}

#[test]
fn scan_missing_directory_is_empty() {
    let missing = std::env::temp_dir().join("resonance_test_audio_probe_does_not_exist_xyz");
    let _ = std::fs::remove_dir_all(&missing);
    assert!(scan_audio_folder(&missing).is_empty());
}
