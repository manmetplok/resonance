//! The `render.mixdown` completion path's WAV-header parsing: the job
//! result's `duration_s` / `sample_rate` come from a bounded read of the
//! written file's RIFF chunk list (headers + `fmt ` fields only, seeking
//! past chunk bodies), never from slurping the whole render into memory
//! on the update thread. Covers canonical PCM, extra chunks before
//! `data`, 24-bit, odd-size chunk padding, the malformed-file fallback,
//! and a sparse multi-hundred-MB `data` chunk that only a header-walk
//! can survive.

use resonance_app::Resonance;
use resonance_audio::types::AudioEvent;
use resonance_control::job::{JobStarted, JobState, JobStatus};
use resonance_control::methods::render::MixdownResult;
use resonance_control::Request;
use serde_json::json;
use std::io::Write;
use std::path::Path;
use crate::common::roundtrip;
use crate::common::app_active_project as app;

fn request(id: i64, method: &str, params: serde_json::Value) -> Request {
    Request::new(id, method, &params).expect("params serialize")
}

fn job_status(app: &mut Resonance, job_id: u64) -> JobStatus {
    roundtrip(app, request(99, "job.status", json!({ "job_id": job_id })))
        .result()
        .expect("job.status succeeds")
}

/// Run a full `render.mixdown` round: start the job, let `write_file`
/// stand in for the engine's render at the target path, deliver the
/// engine's `BounceComplete`, and return the completed job's result —
/// the observable output of the header parse.
fn rendered_result(write_file: impl FnOnce(&Path)) -> MixdownResult {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("mix.wav");

    let mut app = app();
    let started: JobStarted = roundtrip(
        &mut app,
        request(1, "render.mixdown", json!({ "path": target.display().to_string() })),
    )
    .result()
    .expect("job started");
    let job = u64::from(started.job_id);

    write_file(&target);
    app.test_apply_engine_event(AudioEvent::BounceComplete {
        path: target.display().to_string(),
    });

    let status = job_status(&mut app, job);
    assert_eq!(status.state, JobState::Done);
    serde_json::from_value(status.result.expect("done carries a result")).unwrap()
}

// ---------------- hand-rolled RIFF building blocks ----------------

/// Append one chunk, including the word-alignment pad byte after an
/// odd-sized body.
fn chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
}

/// A 16-byte PCM `fmt ` chunk body.
fn fmt_body(channels: u16, sample_rate: u32, bits: u16) -> Vec<u8> {
    let block_align = channels * (bits / 8);
    let byte_rate = sample_rate * u32::from(block_align);
    let mut body = Vec::new();
    body.extend_from_slice(&1u16.to_le_bytes()); // PCM
    body.extend_from_slice(&channels.to_le_bytes());
    body.extend_from_slice(&sample_rate.to_le_bytes());
    body.extend_from_slice(&byte_rate.to_le_bytes());
    body.extend_from_slice(&block_align.to_le_bytes());
    body.extend_from_slice(&bits.to_le_bytes());
    body
}

/// Wrap already-built chunks in a `RIFF…WAVE` container.
fn riff(chunks: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((4 + chunks.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(chunks);
    out
}

// ---------------- parsing ----------------

#[test]
fn standard_pcm_wav_reports_its_duration_and_rate() {
    let result = rendered_result(|path| {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).expect("create wav");
        for _ in 0..44_100 {
            writer.write_sample(0i16).unwrap();
            writer.write_sample(0i16).unwrap();
        }
        writer.finalize().expect("finalize wav");
    });
    assert_eq!(result.sample_rate, 44_100);
    assert!(
        (result.duration_s - 1.0).abs() < 1e-6,
        "44.1k frames at 44.1k -> 1.0s, got {}",
        result.duration_s
    );
}

#[test]
fn extra_chunks_before_data_still_parse() {
    let result = rendered_result(|path| {
        let mut chunks = Vec::new();
        // `fact` and a LIST/INFO chunk ahead of `fmt ` and `data`, as
        // some encoders emit.
        chunk(&mut chunks, b"fact", &22_050u32.to_le_bytes());
        chunk(&mut chunks, b"LIST", b"INFOISFT\x06\x00\x00\x00tests\0");
        chunk(&mut chunks, b"fmt ", &fmt_body(2, 44_100, 16));
        chunk(&mut chunks, b"data", &vec![0u8; 22_050 * 4]);
        std::fs::write(path, riff(&chunks)).expect("write wav");
    });
    assert_eq!(result.sample_rate, 44_100);
    assert!(
        (result.duration_s - 0.5).abs() < 1e-6,
        "22050 frames at 44.1k -> 0.5s, got {}",
        result.duration_s
    );
}

#[test]
fn twenty_four_bit_wav_reports_its_duration() {
    let result = rendered_result(|path| {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).expect("create wav");
        for _ in 0..24_000 {
            writer.write_sample(0i32).unwrap();
            writer.write_sample(0i32).unwrap();
        }
        writer.finalize().expect("finalize wav");
    });
    assert_eq!(result.sample_rate, 48_000);
    assert!(
        (result.duration_s - 0.5).abs() < 1e-6,
        "24k frames at 48k -> 0.5s, got {}",
        result.duration_s
    );
}

#[test]
fn odd_sized_chunk_is_skipped_over_its_pad_byte() {
    let result = rendered_result(|path| {
        let mut chunks = Vec::new();
        // A 5-byte chunk body; without honouring the pad byte the walk
        // lands one byte short of `fmt ` and finds nothing.
        chunk(&mut chunks, b"junk", b"12345");
        chunk(&mut chunks, b"fmt ", &fmt_body(1, 48_000, 16));
        chunk(&mut chunks, b"data", &vec![0u8; 12_000 * 2]);
        std::fs::write(path, riff(&chunks)).expect("write wav");
    });
    assert_eq!(result.sample_rate, 48_000);
    assert!(
        (result.duration_s - 0.25).abs() < 1e-6,
        "12k mono frames at 48k -> 0.25s, got {}",
        result.duration_s
    );
}

// ---------------- malformed files: the fallback ----------------

#[test]
fn malformed_files_fall_back_to_the_engine_rate_and_zero_duration() {
    // Matches the pre-existing behavior for an unparsable render target:
    // the job still completes (the file exists, so the render itself
    // succeeded), with a zero duration and the engine's sample rate —
    // 44.1k on the test app.
    let assert_fallback = |result: MixdownResult, what: &str| {
        assert_eq!(result.sample_rate, 44_100, "{what}: engine-rate fallback");
        assert_eq!(result.duration_s, 0.0, "{what}: zero-duration fallback");
    };

    // Not RIFF at all.
    assert_fallback(
        rendered_result(|path| std::fs::write(path, b"not a wav at all").expect("write")),
        "garbage bytes",
    );
    // Shorter than the 12-byte RIFF/WAVE preamble.
    assert_fallback(
        rendered_result(|path| std::fs::write(path, b"RIFF").expect("write")),
        "truncated preamble",
    );
    // Well-formed container but truncated before any `data` chunk.
    assert_fallback(
        rendered_result(|path| {
            let mut chunks = Vec::new();
            chunk(&mut chunks, b"fmt ", &fmt_body(2, 48_000, 16));
            std::fs::write(path, riff(&chunks)).expect("write");
        }),
        "no data chunk",
    );
}

// ---------------- bounded I/O ----------------

#[test]
fn huge_data_chunk_is_parsed_from_the_header_alone() {
    // One hour of 16-bit stereo 48k: a ~660 MB `data` chunk, created
    // sparsely so only the header ever touches the disk. Correct
    // geometry out of this file means the parser walked headers and
    // seeked — a whole-file read would have had to materialize all of
    // it just to report a duration.
    const DATA_BYTES: u64 = 3_600 * 48_000 * 4;

    let result = rendered_result(|path| {
        let mut header = Vec::new();
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&((36 + DATA_BYTES) as u32).to_le_bytes());
        header.extend_from_slice(b"WAVE");
        chunk(&mut header, b"fmt ", &fmt_body(2, 48_000, 16));
        header.extend_from_slice(b"data");
        header.extend_from_slice(&(DATA_BYTES as u32).to_le_bytes());

        let mut file = std::fs::File::create(path).expect("create wav");
        file.write_all(&header).expect("write header");
        // Extend to the declared size without writing the body.
        file.set_len(header.len() as u64 + DATA_BYTES)
            .expect("extend sparsely");
    });
    assert_eq!(result.sample_rate, 48_000);
    assert!(
        (result.duration_s - 3_600.0).abs() < 1e-6,
        "an hour of 48k stereo -> 3600s, got {}",
        result.duration_s
    );
}
