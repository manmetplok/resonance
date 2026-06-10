//! Audio decoding + resampling utilities shared across the workspace:
//! IR / drum / sample-loading plugin code (WAV bytes in memory) and the
//! engine's clip-import path (arbitrary audio files on disk).
//!
//! Implemented on top of `symphonia`: one reader covers all the raw
//! WAV bit depths the project cares about (8/16/24/32-bit integer,
//! 32/64-bit float) plus every compressed format the workspace
//! `symphonia` features enable (FLAC, MP3, Ogg/Vorbis, AAC, MP4),
//! without per-format branching here. The public API —
//! `decode_wav_stereo`, `decode_wav_channels`, `decode_file`, and the
//! linear resamplers — keeps the signatures downstream crates depend
//! on.

use std::io::Cursor;
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Decoded WAV data split into separate channels.
pub struct WavChannels {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub stereo: bool,
}

/// Decode a WAV file from bytes into stereo interleaved f32 samples,
/// resampled to the target sample rate if necessary.
pub fn decode_wav_stereo(data: &[u8], target_sample_rate: f32) -> Result<Vec<f32>, String> {
    let decoded = decode_to_interleaved(data)?;
    let source_rate = decoded.sample_rate;
    let stereo = to_stereo_interleaved(&decoded.samples, decoded.channels);

    if (source_rate - target_sample_rate).abs() > 1.0 {
        Ok(linear_resample_stereo(
            &stereo,
            source_rate,
            target_sample_rate,
        ))
    } else {
        Ok(stereo)
    }
}

/// Decode a WAV file from bytes into separate left/right channels,
/// resampled to the target sample rate if necessary.
pub fn decode_wav_channels(data: &[u8], target_sample_rate: f32) -> Result<WavChannels, String> {
    let decoded = decode_to_interleaved(data)?;
    let source_rate = decoded.sample_rate;
    let channels = decoded.channels;
    let raw_samples = decoded.samples;
    let frames = raw_samples.len() / channels;

    let (left, right, stereo) = if channels >= 2 {
        let mut l = Vec::with_capacity(frames);
        let mut r = Vec::with_capacity(frames);
        for frame in 0..frames {
            l.push(raw_samples[frame * channels]);
            r.push(raw_samples[frame * channels + 1]);
        }
        (l, r, true)
    } else {
        (raw_samples, Vec::new(), false)
    };

    let needs_resample = (source_rate - target_sample_rate).abs() > 1.0;
    let (left, right) = if needs_resample {
        let l = linear_resample_mono(&left, source_rate, target_sample_rate);
        let r = if stereo {
            linear_resample_mono(&right, source_rate, target_sample_rate)
        } else {
            Vec::new()
        };
        (l, r)
    } else {
        (left, right)
    };

    Ok(WavChannels {
        left,
        right,
        stereo,
    })
}

/// Decode an audio file to stereo interleaved f32 samples at the
/// target sample rate. Returns the samples plus a display name
/// derived from the file stem. Any format the workspace `symphonia`
/// features enable is accepted, not just WAV.
pub fn decode_file(path: &str, target_sample_rate: u32) -> Result<(Vec<f32>, String), String> {
    let path = Path::new(path);
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string();

    let file = std::fs::File::open(path).map_err(|e| format!("Failed to open file: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let decoded = decode_source_to_interleaved(mss, hint, "audio")?;
    let source_rate = decoded.sample_rate;
    let stereo = to_stereo_interleaved(&decoded.samples, decoded.channels);

    let target = target_sample_rate as f32;
    let output = if (source_rate - target).abs() > 1.0 {
        linear_resample_stereo(&stereo, source_rate, target)
    } else {
        stereo
    };

    Ok((output, name))
}

struct Decoded {
    samples: Vec<f32>,
    sample_rate: f32,
    channels: usize,
}

/// Run the input bytes through symphonia's default decoder registry
/// and return the full interleaved `f32` sample stream plus the
/// source rate and channel count.
fn decode_to_interleaved(data: &[u8]) -> Result<Decoded, String> {
    let cursor = Cursor::new(data.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let mut hint = Hint::new();
    hint.with_extension("wav");

    decode_source_to_interleaved(mss, hint, "WAV")
}

/// Shared symphonia probe → decode loop behind both the in-memory WAV
/// API and `decode_file`. `kind` only labels error messages.
fn decode_source_to_interleaved(
    mss: MediaSourceStream,
    hint: Hint,
    kind: &str,
) -> Result<Decoded, String> {
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("{kind} probe error: {e}"))?;

    let track = format
        .first_track_known_codec(TrackType::Audio)
        .ok_or_else(|| format!("{kind} has no decodable track"))?;
    let track_id = track.id;
    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| format!("{kind} track missing audio codec parameters"))?
        .clone();

    let sample_rate = audio_params
        .sample_rate
        .map(|sr| sr as f32)
        .ok_or_else(|| format!("{kind} missing sample rate"))?;
    let channels = audio_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(1)
        .max(1);

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .map_err(|e| format!("{kind} decoder error: {e}"))?;

    let mut samples: Vec<f32> = Vec::new();
    // Per-packet scratch. `copy_to_vec_interleaved` *resizes* its
    // destination to the current packet's sample count rather than
    // appending — using `samples` directly would clobber every prior
    // packet, leaving only the last one (a few hundred frames for a
    // multi-second WAV decoded packet-by-packet). Decode into the
    // scratch and `extend` `samples` from it instead.
    let mut packet_buf: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymphoniaError::IoError(_)) => break,
            Err(e) => return Err(format!("{kind} read packet: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(_)) => break,
            Err(e) => return Err(format!("{kind} decode: {e}")),
        };
        decoded.copy_to_vec_interleaved(&mut packet_buf);
        samples.extend_from_slice(&packet_buf);
    }

    if samples.is_empty() {
        return Err(format!("{kind} decoded 0 samples"));
    }

    Ok(Decoded {
        samples,
        sample_rate,
        channels,
    })
}

fn to_stereo_interleaved(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels == 2 {
        return samples.to_vec();
    }
    if channels > 2 {
        // Take the first two channels only, ignore the rest.
        let frames = samples.len() / channels;
        let mut stereo = Vec::with_capacity(frames * 2);
        for frame in 0..frames {
            stereo.push(samples[frame * channels]);
            stereo.push(samples[frame * channels + 1]);
        }
        return stereo;
    }
    // Mono → duplicate.
    let mut stereo = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        stereo.push(s);
        stereo.push(s);
    }
    stereo
}

/// Linear interpolation resampler for mono audio data.
pub fn linear_resample_mono(input: &[f32], source_rate: f32, target_rate: f32) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
    let ratio = source_rate as f64 / target_rate as f64;
    // Clamp to 1: a heavy downsample of a tiny input truncates to zero
    // samples, silently discarding non-empty audio.
    let target_len = ((input.len() as f64 / ratio) as usize).max(1);
    let mut output = Vec::with_capacity(target_len);

    for i in 0..target_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos as usize;
        let frac = (src_pos - idx as f64) as f32;

        let s0 = input[idx.min(input.len() - 1)];
        let s1 = input[(idx + 1).min(input.len() - 1)];
        output.push(s0 + (s1 - s0) * frac);
    }

    output
}

/// Linear interpolation resampler for stereo interleaved audio data.
pub fn linear_resample_stereo(input: &[f32], source_rate: f32, target_rate: f32) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
    let source_frames = input.len() / 2;
    if source_frames == 0 {
        // A single stray sample is not a full stereo frame.
        return Vec::new();
    }
    let ratio = source_rate as f64 / target_rate as f64;
    // Clamp to 1: a heavy downsample of a tiny input truncates to zero
    // frames, silently discarding non-empty audio.
    let target_frames = ((source_frames as f64 / ratio) as usize).max(1);
    let mut output = Vec::with_capacity(target_frames * 2);

    for i in 0..target_frames {
        let src_pos = i as f64 * ratio;
        let src_idx = src_pos as usize;
        let frac = (src_pos - src_idx as f64) as f32;

        let idx0 = src_idx.min(source_frames.saturating_sub(1));
        let idx1 = (src_idx + 1).min(source_frames.saturating_sub(1));

        let l0 = input[idx0 * 2];
        let r0 = input[idx0 * 2 + 1];
        let l1 = input[idx1 * 2];
        let r1 = input[idx1 * 2 + 1];

        output.push(l0 + (l1 - l0) * frac);
        output.push(r0 + (r1 - r0) * frac);
    }

    output
}

/// Stateful linear resampler for stereo interleaved audio that can
/// be fed in chunks without introducing discontinuities at chunk
/// boundaries. Used by the recording drain loop so takes stream to
/// disk at the engine sample rate without ever materialising the
/// full buffer.
///
/// The algorithm matches [`linear_resample_stereo`] in steady state —
/// for the same total input the emitted samples agree to within the
/// precision of `f64` phase accumulation — but carries a one-frame
/// tail and a fractional phase across calls. Callers should emit a
/// final [`StreamingLinearResampler::flush`] when the input stream
/// ends so the trailing frame isn't lost.
pub struct StreamingLinearResampler {
    /// Ratio = source_rate / target_rate. Each output frame advances
    /// the read head by `ratio` input frames.
    ratio: f64,
    /// Next read position, measured in input frames, as a
    /// floating-point offset into the virtual concatenation of every
    /// chunk the caller has pushed so far.
    next_src_pos: f64,
    /// Total input frames consumed across all prior calls. Used to
    /// translate `next_src_pos` into a chunk-local index.
    consumed_frames: u64,
    /// The last input frame from the previous chunk, retained so that
    /// interpolating at position `consumed_frames - 1 + frac` works
    /// correctly on the first output frame of the next chunk.
    last_frame: Option<[f32; 2]>,
}

impl StreamingLinearResampler {
    pub fn new(source_rate: u32, target_rate: u32) -> Self {
        Self {
            ratio: source_rate as f64 / target_rate as f64,
            next_src_pos: 0.0,
            consumed_frames: 0,
            last_frame: None,
        }
    }

    /// Process a chunk of stereo-interleaved f32 input, appending
    /// resampled stereo frames to `output`. Does not emit the final
    /// partial frame; call [`StreamingLinearResampler::flush`] once
    /// after the last chunk to drain it.
    pub fn process(&mut self, input: &[f32], output: &mut Vec<f32>) {
        let chunk_frames = input.len() / 2;
        if chunk_frames == 0 {
            return;
        }

        // Total frames available in the virtual stream up to the end
        // of this chunk.
        let total_frames = self.consumed_frames + chunk_frames as u64;

        // Emit output frames while both neighbours of the
        // interpolation window are fully inside the data we've seen
        // so far. The right neighbour lives at index `src_idx + 1`,
        // so we stop when `src_idx + 1 >= total_frames`, i.e. when
        // `src_idx >= total_frames - 1`.
        loop {
            let src_idx_f = self.next_src_pos.floor();
            if src_idx_f < 0.0 {
                // Shouldn't happen — phase is monotonically
                // non-negative — but guard defensively.
                self.next_src_pos += self.ratio;
                continue;
            }
            let src_idx = src_idx_f as u64;
            if src_idx + 1 >= total_frames {
                break;
            }
            let frac = (self.next_src_pos - src_idx_f) as f32;

            let [l0, r0] = self.sample_at(src_idx, input);
            let [l1, r1] = self.sample_at(src_idx + 1, input);

            output.push(l0 + (l1 - l0) * frac);
            output.push(r0 + (r1 - r0) * frac);

            self.next_src_pos += self.ratio;
        }

        // Retain the last input frame for the next chunk so that a
        // read at index `total_frames - 1` still resolves.
        self.last_frame = Some([
            input[(chunk_frames - 1) * 2],
            input[(chunk_frames - 1) * 2 + 1],
        ]);
        self.consumed_frames = total_frames;
    }

    /// Emit any trailing output frame that wasn't produced during
    /// [`StreamingLinearResampler::process`] because the right
    /// neighbour would have fallen past the end of the stream. Uses
    /// the retained last frame for both neighbours (equivalent to
    /// clamping the read position to the final input frame).
    pub fn flush(&mut self, output: &mut Vec<f32>) {
        let Some(last) = self.last_frame else {
            return;
        };
        let total_frames = self.consumed_frames;
        if total_frames == 0 {
            return;
        }
        loop {
            let src_idx_f = self.next_src_pos.floor();
            let src_idx = src_idx_f as u64;
            if src_idx >= total_frames {
                break;
            }
            // Both neighbours clamped to the last valid frame.
            output.push(last[0]);
            output.push(last[1]);
            self.next_src_pos += self.ratio;
        }
    }

    /// Resolve a frame at a virtual index into either the retained
    /// tail frame (if it points at `consumed_frames - 1`) or the
    /// current chunk (otherwise). Assumes the caller has already
    /// checked that `idx < consumed_frames + chunk_frames`.
    #[inline]
    fn sample_at(&self, idx: u64, chunk: &[f32]) -> [f32; 2] {
        if idx < self.consumed_frames {
            // Only the single most-recent past frame is retained,
            // so any earlier reference is a bug. Clamp as a safety
            // net — the first `process` call starts at `0` anyway.
            if let Some(last) = self.last_frame {
                return last;
            }
            return [chunk[0], chunk[1]];
        }
        let local = (idx - self.consumed_frames) as usize;
        [chunk[local * 2], chunk[local * 2 + 1]]
    }
}

