//! The RIFF/WAVE chunk walk behind clip loading (`io::wav`, ba todo
//! #1260).
//!
//! `locate_wav_float_data` is what stands between a file on disk and the
//! mixer reading `&[f32]` straight out of a memory mapping, so every one
//! of its rejections matters: a header it misreads becomes an
//! out-of-bounds slice over mapped memory. It is a pure function over
//! bytes, so these tests build the headers by hand — valid float WAV,
//! the WAVE_FORMAT_EXTENSIBLE shape `hound` writes, and the malformed
//! cases — with no filesystem involved.

use resonance_audio::__test_support::locate_wav_float_data;

const SR: u32 = 48_000;

/// A plain `WAVE_FORMAT_IEEE_FLOAT` fmt chunk body (16 bytes).
fn fmt_float(channels: u16, sample_rate: u32, bits: u16) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&3u16.to_le_bytes()); // wFormatTag = IEEE float
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&sample_rate.to_le_bytes());
    v.extend_from_slice(&(sample_rate * channels as u32 * bits as u32 / 8).to_le_bytes());
    v.extend_from_slice(&(channels * bits / 8).to_le_bytes()); // block align
    v.extend_from_slice(&bits.to_le_bytes());
    v
}

/// The 40-byte `WAVE_FORMAT_EXTENSIBLE` fmt chunk `hound` writes for
/// float WAVs: outer tag 0xFFFE, real format code in the SubFormat GUID.
fn fmt_extensible(sub_format: u16) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&0xFFFEu16.to_le_bytes()); // wFormatTag = EXTENSIBLE
    v.extend_from_slice(&2u16.to_le_bytes()); // channels
    v.extend_from_slice(&SR.to_le_bytes());
    v.extend_from_slice(&(SR * 2 * 4).to_le_bytes());
    v.extend_from_slice(&8u16.to_le_bytes()); // block align
    v.extend_from_slice(&32u16.to_le_bytes()); // bits per sample
    v.extend_from_slice(&22u16.to_le_bytes()); // cbSize
    v.extend_from_slice(&32u16.to_le_bytes()); // valid bits
    v.extend_from_slice(&3u32.to_le_bytes()); // channel mask
    v.extend_from_slice(&sub_format.to_le_bytes()); // SubFormat GUID head
    v.extend_from_slice(&[0u8; 14]); // rest of the GUID
    assert_eq!(v.len(), 40);
    v
}

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(id);
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(body);
    // RIFF chunks are word-aligned.
    if body.len() % 2 == 1 {
        v.push(0);
    }
    v
}

/// Wrap chunk bytes in a RIFF/WAVE container.
fn riff(chunks: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&((chunks.len() + 4) as u32).to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(chunks);
    v
}

/// A complete, valid stereo float WAV with `frames` frames of silence.
fn valid_wav(frames: usize) -> Vec<u8> {
    let mut body = chunk(b"fmt ", &fmt_float(2, SR, 32));
    body.extend_from_slice(&chunk(b"data", &vec![0u8; frames * 2 * 4]));
    riff(&body)
}

#[test]
fn valid_float_wav_locates_data_chunk() {
    let bytes = valid_wav(64);
    let found = locate_wav_float_data(&bytes).expect("valid float WAV must parse");

    // 12-byte RIFF/WAVE header + 8-byte fmt header + 16-byte fmt body +
    // 8-byte data header.
    assert_eq!(found.data_offset_bytes, 44);
    assert_eq!(found.data_len_bytes, 64 * 2 * 4);
    assert_eq!(found.sample_rate, SR);
    assert_eq!(found.stereo_frame_count().unwrap(), 64);
    // The reported span really is inside the file.
    assert!(found.data_offset_bytes + found.data_len_bytes <= bytes.len());
}

#[test]
fn extensible_float_wav_reads_the_subformat_guid() {
    let mut body = chunk(b"fmt ", &fmt_extensible(3));
    body.extend_from_slice(&chunk(b"data", &[0u8; 8 * 8]));
    let found = locate_wav_float_data(&riff(&body)).expect("hound-style extensible WAV must parse");

    assert_eq!(found.sample_rate, SR);
    assert_eq!(found.stereo_frame_count().unwrap(), 8);
}

#[test]
fn extensible_pcm_subformat_is_rejected() {
    let mut body = chunk(b"fmt ", &fmt_extensible(1)); // WAVE_FORMAT_PCM
    body.extend_from_slice(&chunk(b"data", &[0u8; 32]));
    let err = locate_wav_float_data(&riff(&body)).expect_err("integer PCM is not mappable");
    assert!(err.contains("unsupported format code 1"), "{err}");
}

#[test]
fn extensible_fmt_chunk_shorter_than_the_guid_is_rejected() {
    // 0xFFFE outer tag but only a 16-byte body: the SubFormat GUID the
    // parser must read is not there at all.
    let mut short = fmt_extensible(3);
    short.truncate(16);
    let mut body = chunk(b"fmt ", &short);
    body.extend_from_slice(&chunk(b"data", &[0u8; 32]));
    let err = locate_wav_float_data(&riff(&body)).expect_err("truncated extensible fmt");
    assert!(err.contains("extensible fmt chunk too small"), "{err}");
}

#[test]
fn chunks_before_fmt_and_odd_sized_chunks_are_skipped() {
    // A 5-byte LIST chunk (odd, so word-padded) ahead of fmt: the walk
    // must land on the fmt chunk and then on data, not drift by a byte.
    let mut body = chunk(b"LIST", b"INFOx");
    body.extend_from_slice(&chunk(b"fmt ", &fmt_float(2, 44_100, 32)));
    body.extend_from_slice(&chunk(b"data", &[0u8; 16 * 8]));
    let found = locate_wav_float_data(&riff(&body)).expect("padded chunk must not desync the walk");

    assert_eq!(found.sample_rate, 44_100);
    assert_eq!(found.stereo_frame_count().unwrap(), 16);
}

#[test]
fn garbage_and_truncated_headers_are_rejected() {
    let cases: [(&[u8], &str); 4] = [
        (b"", "file too short"),
        (b"RIFF\x04\x00\x00\x00", "file too short"),
        (b"NOPE\x04\x00\x00\x00WAVE", "missing RIFF header"),
        (b"RIFF\x04\x00\x00\x00WAVX", "not a WAVE file"),
    ];
    for (bytes, expected) in cases {
        let err = locate_wav_float_data(bytes).expect_err("must reject");
        assert!(err.contains(expected), "for {bytes:?}: {err}");
    }
}

#[test]
fn missing_data_chunk_is_rejected() {
    let body = chunk(b"fmt ", &fmt_float(2, SR, 32));
    let err = locate_wav_float_data(&riff(&body)).expect_err("no data chunk");
    assert!(err.contains("no data chunk found"), "{err}");
}

#[test]
fn data_chunk_before_fmt_is_rejected() {
    let mut body = chunk(b"data", &[0u8; 32]);
    body.extend_from_slice(&chunk(b"fmt ", &fmt_float(2, SR, 32)));
    let err = locate_wav_float_data(&riff(&body)).expect_err("data before fmt");
    assert!(err.contains("data chunk before fmt chunk"), "{err}");
}

#[test]
fn a_chunk_that_overruns_the_file_is_rejected() {
    // Declared data size far larger than the bytes actually present —
    // the case that would otherwise slice past the end of the mapping.
    let mut bytes = riff(&chunk(b"fmt ", &fmt_float(2, SR, 32)));
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&1_000_000u32.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 16]);
    let err = locate_wav_float_data(&bytes).expect_err("overrunning chunk");
    assert!(err.contains("overruns file"), "{err}");
}

#[test]
fn mono_and_non_float_and_zero_rate_fmt_chunks_are_rejected() {
    let cases: [(Vec<u8>, &str); 3] = [
        (fmt_float(1, SR, 32), "expected stereo, got 1 channels"),
        (fmt_float(2, SR, 16), "expected 32-bit float, got 16 bits"),
        (fmt_float(2, 0, 32), "zero sample rate"),
    ];
    for (fmt, expected) in cases {
        let mut body = chunk(b"fmt ", &fmt);
        body.extend_from_slice(&chunk(b"data", &[0u8; 32]));
        let err = locate_wav_float_data(&riff(&body)).expect_err("must reject");
        assert!(err.contains(expected), "{err}");
    }
}

#[test]
fn fmt_chunk_too_small_is_rejected() {
    let mut body = chunk(b"fmt ", &[0u8; 12]);
    body.extend_from_slice(&chunk(b"data", &[0u8; 32]));
    let err = locate_wav_float_data(&riff(&body)).expect_err("stub fmt chunk");
    assert!(err.contains("fmt chunk too small"), "{err}");
}

#[test]
fn a_data_chunk_that_is_not_whole_stereo_frames_is_rejected() {
    let mut body = chunk(b"fmt ", &fmt_float(2, SR, 32));
    // 12 bytes = 1.5 stereo f32 frames.
    body.extend_from_slice(&chunk(b"data", &[0u8; 12]));
    let found = locate_wav_float_data(&riff(&body)).expect("header itself is well-formed");
    let err = found
        .stereo_frame_count()
        .expect_err("half a frame is not loadable");
    assert!(err.contains("not a multiple of stereo f32 frames"), "{err}");
}

/// A `data` chunk that is only 2-byte aligned must still load and play
/// (review follow-up to ba todo #1260).
///
/// RIFF requires chunks to be *word* aligned, not dword, so an
/// odd-length chunk ahead of `data` — a `LIST`/`INFO` block with an
/// odd-length string, which several DAWs write — leaves the samples on a
/// 2-byte boundary. `ClipSource::as_frames` casts the mapped bytes to
/// `&[f32]` with `bytemuck::cast_slice`, which PANICS on misalignment,
/// and it runs on the audio thread. `chunks_before_fmt_and_odd_sized_chunks_are_skipped`
/// blesses exactly this layout as valid but stops at the parser, so it
/// never reaches the cast.
#[test]
fn a_two_byte_aligned_data_chunk_loads_without_panicking() {
    use resonance_audio::types::ClipSource;

    // 5-byte LIST (padded to 6) puts the data body at offset 58, and
    // 58 % 4 == 2.
    let mut body = chunk(b"LIST", b"INFOx");
    body.extend_from_slice(&chunk(b"fmt ", &fmt_float(2, 48_000, 32)));
    let samples: Vec<f32> = (0..32).map(|i| i as f32 * 0.01).collect();
    body.extend_from_slice(&chunk(b"data", bytemuck::cast_slice(&samples)));
    let file = riff(&body);

    let found = locate_wav_float_data(&file).expect("layout is valid RIFF");
    assert_eq!(
        found.data_offset_bytes % std::mem::align_of::<f32>(),
        2,
        "this fixture is only meaningful while the data chunk is misaligned"
    );

    let dir = std::env::temp_dir().join("resonance-wav-align-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("misaligned.wav");
    std::fs::write(&path, &file).expect("write fixture");

    let source = ClipSource::open_wav(&path).expect("a misaligned WAV must still load");
    // The cast that used to panic.
    let frames = source.as_frames();
    assert_eq!(frames.len(), samples.len());
    assert_eq!(frames, samples.as_slice(), "samples must survive the copy");

    let _ = std::fs::remove_file(&path);
}
