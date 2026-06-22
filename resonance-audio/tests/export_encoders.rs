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
//! * MP3 (with the `mp3` feature) round-trips CBR and VBR through
//!   `libmp3lame` — symphonia decodes a valid stereo stream at the engine
//!   rate whose energy survives the lossy pass (todo #653).
//! * Opus (with the `opus` feature) round-trips Music and Voice through
//!   `libopus`, muxed into Ogg — the reference libopus decoder reconstructs
//!   a stereo stream whose energy survives the lossy pass, and the export
//!   resampler feeds the 48 kHz-only codec from a non-48 kHz engine rate
//!   (todo #651). symphonia is not used to decode Opus: this workspace's
//!   symphonia build registers no Opus codec, so libopus (the reference
//!   decoder) verifies the file instead.
//! * MP3 and/or Opus report `EncoderUnavailable` and leave no file behind in
//!   a build with the corresponding feature switched off.

use std::path::{Path, PathBuf};

use resonance_audio::__test_support::encode_buffer_for_test;
#[cfg(feature = "mp3")]
use resonance_audio::types::Mp3Rate;
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
    test_mix_at(ENGINE_SR)
}

/// Like [`test_mix`] but rendered at `sample_rate` and ~0.25 s long, so a
/// non-48 kHz rate can exercise the export resampler.
#[allow(dead_code)] // Only the Opus tests render at a non-default rate.
fn test_mix_at(sample_rate: u32) -> Vec<f32> {
    let frames = (sample_rate / 4) as usize;
    let mut buf = Vec::with_capacity(frames * 2);
    for n in 0..frames {
        let t = n as f32 / sample_rate as f32;
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

    // MP3 and Opus are available when their feature is compiled in, so each
    // only joins this set in a feature-off build. With the default features
    // (both on) there is nothing to check and the loop is a no-op.
    #[allow(unused_mut)]
    let mut cases: Vec<(&str, ExportFormat)> = Vec::new();
    #[cfg(not(feature = "opus"))]
    cases.push((
        "export.opus",
        ExportFormat::Opus {
            bitrate_kbps: 192,
            optimize: resonance_audio::types::OpusOptimize::Music,
        },
    ));
    #[cfg(not(feature = "mp3"))]
    cases.push((
        "export.mp3",
        ExportFormat::Mp3 {
            mode: resonance_audio::types::Mp3Rate::Cbr,
            bitrate_kbps: 320,
        },
    ));

    for (name, format) in cases {
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

/// Root-mean-square level of an interleaved buffer — an energy proxy that
/// lets a lossy round trip (MP3 / Opus) be checked without a sample-exact
/// comparison (which a lossy codec can never satisfy).
#[cfg(any(feature = "mp3", feature = "opus"))]
fn rms(buf: &[f32]) -> f32 {
    if buf.is_empty() {
        return 0.0;
    }
    (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
}

/// Encode the test mix as MP3, decode it back through symphonia, and assert
/// the file is a valid stereo MP3 at the engine rate whose duration and
/// energy survive the lossy pass.
#[cfg(feature = "mp3")]
fn assert_mp3_round_trip(mode: Mp3Rate, bitrate_kbps: u32, tag: &str) {
    let dir = tempdir(tag);
    let mix = test_mix();
    let out = dir.join(format!("{tag}.mp3"));

    let bytes = encode_buffer_for_test(
        &ExportFormat::Mp3 { mode, bitrate_kbps },
        &ExportMetadata::default(),
        ENGINE_SR,
        &mix,
        &out,
    )
    .expect("encode mp3");
    assert!(bytes > 0, "a non-empty file is reported");
    assert_eq!(
        std::fs::metadata(&out).unwrap().len(),
        bytes,
        "reported byte size matches the file on disk"
    );

    // symphonia must accept the stream as MP3 at the engine sample rate.
    let info = probe_audio_file(&out).expect("probe mp3");
    assert_eq!(info.format, AudioFormat::Mp3);
    assert_eq!(info.sample_rate, ENGINE_SR, "mp3 keeps the engine sample rate");
    assert_eq!(info.channels, 2, "stereo");

    // Decode the whole stream: MP3 adds encoder delay + padding, so the
    // frame count is close to the input but not exact.
    let decoded = decode_native(&out);
    assert!(!decoded.is_empty(), "decoded audio is non-empty");
    let decoded_frames = (decoded.len() / 2) as i64;
    let diff = (decoded_frames - FRAMES as i64).abs();
    assert!(
        diff <= 3000,
        "decoded frames {decoded_frames} ~ {FRAMES} (diff {diff})"
    );

    // Signal energy survives the lossy encode (catches a silent or
    // wrongly-scaled encode — e.g. if full-scale f32 were misinterpreted).
    let (in_rms, out_rms) = (rms(&mix), rms(&decoded));
    let ratio = out_rms / in_rms;
    assert!(
        (0.5..2.0).contains(&ratio),
        "energy preserved: in {in_rms:.4} out {out_rms:.4} ratio {ratio:.3}"
    );
}

#[cfg(feature = "mp3")]
#[test]
fn mp3_cbr_round_trips() {
    assert_mp3_round_trip(Mp3Rate::Cbr, 320, "mp3-cbr-320");
    assert_mp3_round_trip(Mp3Rate::Cbr, 128, "mp3-cbr-128");
}

#[cfg(feature = "mp3")]
#[test]
fn mp3_vbr_round_trips() {
    assert_mp3_round_trip(Mp3Rate::Vbr, 192, "mp3-vbr-192");
}

/// Decode an Ogg-Opus file with the reference libopus decoder (symphonia
/// has no Opus codec in this workspace), returning interleaved stereo f32.
/// Skips the two header packets (`OpusHead` / `OpusTags`) and decodes the
/// rest at 48 kHz.
#[cfg(feature = "opus")]
fn decode_opus(path: &Path) -> Vec<f32> {
    use std::io::Cursor;
    let bytes = std::fs::read(path).expect("read opus file");
    let mut reader = ogg::PacketReader::new(Cursor::new(bytes));
    let mut decoder = opus::Decoder::new(48_000, opus::Channels::Stereo).expect("opus decoder");
    let mut out: Vec<f32> = Vec::new();
    // Largest Opus frame is 120 ms = 5760 samples/channel; size the scratch
    // buffer for that worst case (our encoder uses 20 ms frames).
    let mut scratch = vec![0f32; 5760 * 2];
    let mut packet_idx = 0usize;
    while let Some(packet) = reader.read_packet().expect("read ogg packet") {
        if packet_idx < 2 {
            packet_idx += 1; // OpusHead, then OpusTags.
            continue;
        }
        let frames = decoder
            .decode_float(&packet.data, &mut scratch, false)
            .expect("decode opus packet");
        out.extend_from_slice(&scratch[..frames * 2]);
        packet_idx += 1;
    }
    out
}

/// Encode the test mix (rendered at `engine_sr`) as Ogg-Opus, decode it back
/// through libopus, and assert it is a valid Ogg stream at 48 kHz whose
/// duration and energy survive the lossy pass.
#[cfg(feature = "opus")]
fn assert_opus_round_trip(
    optimize: resonance_audio::types::OpusOptimize,
    bitrate_kbps: u32,
    engine_sr: u32,
    tag: &str,
) {
    let dir = tempdir(tag);
    // Render the source at `engine_sr` so a non-48k rate exercises the
    // export resampler that feeds the 48 kHz-only codec.
    let mix = test_mix_at(engine_sr);
    let out = dir.join(format!("{tag}.opus"));

    let bytes = encode_buffer_for_test(
        &ExportFormat::Opus {
            bitrate_kbps,
            optimize,
        },
        &ExportMetadata::default(),
        engine_sr,
        &mix,
        &out,
    )
    .expect("encode opus");
    assert!(bytes > 0, "a non-empty file is reported");
    assert_eq!(
        std::fs::metadata(&out).unwrap().len(),
        bytes,
        "reported byte size matches the file on disk"
    );

    // The file is a real Ogg stream whose first logical page starts an
    // OpusHead identification header.
    let raw = std::fs::read(&out).unwrap();
    assert_eq!(&raw[..4], b"OggS", "file is an Ogg container");
    assert!(
        raw.windows(8).any(|w| w == b"OpusHead"),
        "Ogg stream carries an OpusHead header"
    );

    // Decoding via libopus yields a non-empty stereo stream whose length is
    // close to the resampled-and-frame-padded input (Opus pads the tail to a
    // whole 20 ms frame and adds pre-skip priming samples).
    let decoded = decode_opus(&out);
    assert!(!decoded.is_empty(), "decoded audio is non-empty");
    let decoded_frames = (decoded.len() / 2) as i64;
    let expected_frames = (mix.len() / 2) as f64 * 48_000.0 / engine_sr as f64;
    let diff = (decoded_frames - expected_frames.round() as i64).abs();
    assert!(
        diff <= 2_000,
        "decoded frames {decoded_frames} ~ {expected_frames:.0} (diff {diff})"
    );

    // Signal energy survives the lossy encode (catches a silent or
    // wrongly-scaled encode). Resample to the engine rate first so both
    // buffers are measured at the same rate.
    let (in_rms, out_rms) = (rms(&mix), rms(&decoded));
    let ratio = out_rms / in_rms;
    assert!(
        (0.5..2.0).contains(&ratio),
        "energy preserved: in {in_rms:.4} out {out_rms:.4} ratio {ratio:.3}"
    );
}

#[cfg(feature = "opus")]
#[test]
fn opus_music_and_voice_round_trip() {
    use resonance_audio::types::OpusOptimize;
    // Both application hints, at the export UI's bitrates, from the 48 kHz
    // engine rate (no resampling): the file decodes and energy survives.
    assert_opus_round_trip(OpusOptimize::Music, 160, ENGINE_SR, "opus-music-160");
    assert_opus_round_trip(OpusOptimize::Voice, 96, ENGINE_SR, "opus-voice-96");
    assert_opus_round_trip(OpusOptimize::Music, 256, ENGINE_SR, "opus-music-256");
}

#[cfg(feature = "opus")]
#[test]
fn opus_resamples_non_48k_engine_rate() {
    use resonance_audio::types::OpusOptimize;
    // A 44.1 kHz engine rate must be resampled to the codec's 48 kHz by the
    // shared export resampler before the Opus sink.
    assert_opus_round_trip(OpusOptimize::Music, 160, 44_100, "opus-resample-44k");
}
