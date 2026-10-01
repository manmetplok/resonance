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
//! resamplers (band-limited, see `resample.rs`) — keeps the signatures
//! downstream crates depend on.

use std::io::Cursor;
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use thiserror::Error;

/// Failure decoding a WAV (or, for [`decode_file`], any workspace-enabled
/// `symphonia` format) into samples. `kind` labels which entry point failed
/// ("WAV" for the in-memory decoders, "audio" for [`decode_file`]) — the same
/// role the string of that name played in the pre-thiserror error messages.
#[derive(Debug, Error)]
pub enum WavDecodeError {
    #[error("Failed to open file: {0}")]
    Open(#[source] std::io::Error),
    #[error("{kind} probe error: {source}")]
    Probe {
        kind: &'static str,
        #[source]
        source: SymphoniaError,
    },
    #[error("{kind} has no decodable track")]
    NoTrack { kind: &'static str },
    #[error("{kind} track missing audio codec parameters")]
    MissingCodecParams { kind: &'static str },
    #[error("{kind} missing sample rate")]
    MissingSampleRate { kind: &'static str },
    #[error("{kind} decoder error: {source}")]
    Decoder {
        kind: &'static str,
        #[source]
        source: SymphoniaError,
    },
    #[error("{kind} read packet: {source}")]
    ReadPacket {
        kind: &'static str,
        #[source]
        source: SymphoniaError,
    },
    #[error("{kind} decode: {source}")]
    Decode {
        kind: &'static str,
        #[source]
        source: SymphoniaError,
    },
    #[error("{kind} decoded 0 samples")]
    Empty { kind: &'static str },
}

/// Decoded WAV data split into separate channels.
pub struct WavChannels {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub stereo: bool,
}

/// Decode a WAV file from bytes into stereo interleaved f32 samples,
/// resampled to the target sample rate if necessary.
pub fn decode_wav_stereo(data: &[u8], target_sample_rate: f32) -> Result<Vec<f32>, WavDecodeError> {
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

/// Decoded audio that keeps its channel layout: one channel for a mono
/// file, two (interleaved) for anything wider. A mono file is *not*
/// duplicated to stereo, so it costs half the memory; a reader that wants
/// stereo plays the one channel on both sides.
pub struct DecodedAudio {
    /// Interleaved samples, `channels` per frame, at the target rate.
    pub samples: Vec<f32>,
    /// 1 or 2. Files with more than two channels keep their first two.
    pub channels: usize,
}

impl DecodedAudio {
    /// Number of frames held.
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1)
    }
}

/// Decode a WAV file from bytes, resampled to `target_sample_rate`, keeping
/// mono as mono (see [`DecodedAudio`]).
///
/// Each channel comes out bit-identical to the matching channel of
/// [`decode_wav_stereo`]: the resampler filters every channel on its own,
/// so a mono file read on both sides equals its duplicated-stereo decode.
/// Takes the bytes by value so the decoder does not copy them again.
pub fn decode_wav_native(
    data: Vec<u8>,
    target_sample_rate: f32,
) -> Result<DecodedAudio, WavDecodeError> {
    let cursor = Cursor::new(data);
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("wav");
    let decoded = decode_source_to_interleaved(mss, hint, "WAV")?;
    let source_rate = decoded.sample_rate;
    let resample = (source_rate - target_sample_rate).abs() > 1.0;
    if decoded.channels == 1 {
        let samples = if resample {
            linear_resample_mono(&decoded.samples, source_rate, target_sample_rate)
        } else {
            decoded.samples
        };
        return Ok(DecodedAudio {
            samples,
            channels: 1,
        });
    }
    let stereo = if decoded.channels == 2 {
        decoded.samples
    } else {
        to_stereo_interleaved(&decoded.samples, decoded.channels)
    };
    let samples = if resample {
        linear_resample_stereo(&stereo, source_rate, target_sample_rate)
    } else {
        stereo
    };
    Ok(DecodedAudio {
        samples,
        channels: 2,
    })
}

/// Decode a WAV file from bytes into separate left/right channels,
/// resampled to the target sample rate if necessary.
pub fn decode_wav_channels(
    data: &[u8],
    target_sample_rate: f32,
) -> Result<WavChannels, WavDecodeError> {
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
pub fn decode_file(
    path: &str,
    target_sample_rate: u32,
) -> Result<(Vec<f32>, String), WavDecodeError> {
    let path = Path::new(path);
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string();

    let file = std::fs::File::open(path).map_err(WavDecodeError::Open)?;
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
fn decode_to_interleaved(data: &[u8]) -> Result<Decoded, WavDecodeError> {
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
    kind: &'static str,
) -> Result<Decoded, WavDecodeError> {
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|source| WavDecodeError::Probe { kind, source })?;

    let track = format
        .first_track_known_codec(TrackType::Audio)
        .ok_or(WavDecodeError::NoTrack { kind })?;
    let track_id = track.id;
    // The container's declared length, when present. The final FLAC frame is
    // padded to a full block by some encoders (see resonance-audio's export
    // sink), so the decoder can emit a few silent samples past this count;
    // honoring it trims that padding, matching spec-compliant decoders.
    let declared_frames = track.num_frames;
    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or(WavDecodeError::MissingCodecParams { kind })?
        .clone();

    let sample_rate = audio_params
        .sample_rate
        .map(|sr| sr as f32)
        .ok_or(WavDecodeError::MissingSampleRate { kind })?;
    let channels = audio_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(1)
        .max(1);

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .map_err(|source| WavDecodeError::Decoder { kind, source })?;

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
            Err(source) => return Err(WavDecodeError::ReadPacket { kind, source }),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(_)) => break,
            Err(source) => return Err(WavDecodeError::Decode { kind, source }),
        };
        decoded.copy_to_vec_interleaved(&mut packet_buf);
        samples.extend_from_slice(&packet_buf);
    }

    if samples.is_empty() {
        return Err(WavDecodeError::Empty { kind });
    }

    // Drop any samples past the container's declared frame count (encoder
    // block-padding on the last frame). Never extends the buffer.
    if let Some(frames) = declared_frames {
        let limit = frames as usize * channels;
        if limit > 0 && limit < samples.len() {
            samples.truncate(limit);
        }
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

/// Sample-rate conversion for mono audio. Despite the historical name
/// this is the band-limited windowed-sinc resampler of
/// [`crate::resample`] (LIB-01), not linear interpolation.
pub fn linear_resample_mono(input: &[f32], source_rate: f32, target_rate: f32) -> Vec<f32> {
    crate::resample::resample_mono(input, source_rate, target_rate)
}

/// Sample-rate conversion for stereo interleaved audio. Despite the
/// historical name this is the band-limited windowed-sinc resampler of
/// [`crate::resample`] (LIB-01), not linear interpolation.
pub fn linear_resample_stereo(input: &[f32], source_rate: f32, target_rate: f32) -> Vec<f32> {
    crate::resample::resample_stereo(input, source_rate, target_rate)
}

/// Historical name of [`crate::resample::StreamingResampler`], the
/// band-limited streaming resampler (LIB-01) used by the recording drain
/// and the cpal monitor path.
pub type StreamingLinearResampler = crate::resample::StreamingResampler;
