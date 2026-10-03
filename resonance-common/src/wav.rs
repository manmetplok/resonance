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
    /// The file's own sample rate, before any resampling to the target.
    /// An impulse-response caller needs it: resampling keeps unit DC
    /// gain per sample, but an IR's gain is the *sum* of its taps, which
    /// the caller rescales by `source_rate / target_rate`.
    pub source_rate: f32,
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

// ---------------------------------------------------------------------------
// Head + tail decoding (drums-plugin-rework.md E14: disk streaming).
// ---------------------------------------------------------------------------

/// How a WAV's `data` chunk stores one sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcmEncoding {
    U8,
    S16,
    S24,
    S32,
    F32,
    F64,
}

impl PcmEncoding {
    /// Bytes per sample.
    pub fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::S16 => 2,
            Self::S24 => 3,
            Self::S32 | Self::F32 => 4,
            Self::F64 => 8,
        }
    }

    /// One little-endian sample as `f32`, by the same arithmetic
    /// symphonia's sample conversion uses (so the floats match its
    /// decode bit for bit — which [`decode_wav_split`] also checks, per
    /// file, before it trusts a tail to this).
    #[inline]
    fn to_f32(self, b: &[u8]) -> f32 {
        match self {
            Self::U8 => (b[0] as f32 / 128.0) - 1.0,
            Self::S16 => i16::from_le_bytes([b[0], b[1]]) as f32 / 32_768.0,
            Self::S24 => {
                let v = i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8;
                v as f32 / 8_388_608.0
            }
            Self::S32 => {
                (i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64 / 2_147_483_648.0) as f32
            }
            Self::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            Self::F64 => {
                f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f32
            }
        }
    }
}

/// Where a plain-PCM WAV keeps its samples, read from its header: enough
/// to read any frame straight from the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavPcmLayout {
    /// Byte offset of the first frame.
    pub data_offset: u64,
    /// Whole frames in the `data` chunk (clamped to the bytes present).
    pub frames: u64,
    pub channels: u16,
    pub sample_rate: u32,
    pub encoding: PcmEncoding,
    /// Bytes per frame.
    pub block_align: u16,
}

impl WavPcmLayout {
    /// Parse the RIFF/WAVE header of `bytes` (the whole file). `None` for
    /// anything but integer or float PCM in a well-formed `fmt ` and
    /// `data` chunk — such a file is simply never streamed.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let u16_at = |at: usize| -> Option<u16> {
            Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
        };
        let u32_at = |at: usize| -> Option<u32> {
            Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
        };
        if bytes.get(0..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
            return None;
        }
        let mut fmt: Option<(u16, u16, u32, u16, u16)> = None;
        let mut at = 12usize;
        while at + 8 <= bytes.len() {
            let id = &bytes[at..at + 4];
            let size = u32_at(at + 4)? as usize;
            let body = at + 8;
            if id == b"fmt " {
                let mut tag = u16_at(body)?;
                let channels = u16_at(body + 2)?;
                let rate = u32_at(body + 4)?;
                let block_align = u16_at(body + 12)?;
                let bits = u16_at(body + 14)?;
                if tag == 0xFFFE {
                    // WAVE_FORMAT_EXTENSIBLE: the sub-format GUID opens
                    // with the real tag.
                    if size < 40 {
                        return None;
                    }
                    // A narrower valid width than the container would
                    // need masking; leave such files resident.
                    if u16_at(body + 18)? != bits {
                        return None;
                    }
                    tag = u16_at(body + 24)?;
                }
                fmt = Some((tag, channels, rate, block_align, bits));
            } else if id == b"data" {
                let (tag, channels, rate, block_align, bits) = fmt?;
                let encoding = match (tag, bits) {
                    (1, 8) => PcmEncoding::U8,
                    (1, 16) => PcmEncoding::S16,
                    (1, 24) => PcmEncoding::S24,
                    (1, 32) => PcmEncoding::S32,
                    (3, 32) => PcmEncoding::F32,
                    (3, 64) => PcmEncoding::F64,
                    _ => return None,
                };
                if channels == 0
                    || rate == 0
                    || block_align as usize != channels as usize * encoding.bytes()
                {
                    return None;
                }
                let present = bytes.len().saturating_sub(body).min(size);
                return Some(Self {
                    data_offset: body as u64,
                    frames: (present / block_align as usize) as u64,
                    channels,
                    sample_rate: rate,
                    encoding,
                    block_align,
                });
            }
            at = body.checked_add(size)?.checked_add(size & 1)?;
        }
        None
    }

    /// Convert whole raw frames to interleaved `f32`, keeping `keep`
    /// channels (1: the first; 2: the first two), appended to `out`.
    pub fn convert(&self, raw: &[u8], keep: usize, out: &mut Vec<f32>) {
        let width = self.encoding.bytes();
        for frame in raw.chunks_exact(self.block_align as usize) {
            for ch in 0..keep {
                out.push(self.encoding.to_f32(&frame[ch * width..]));
            }
        }
    }

    /// True when converting the whole `data` chunk of `bytes` gives
    /// exactly `decoded` (every channel, interleaved, bit for bit) — the
    /// check that a tail read through this layout matches the decode.
    fn matches(&self, bytes: &[u8], decoded: &[f32]) -> bool {
        let channels = self.channels as usize;
        if decoded.len() as u64 != self.frames * channels as u64 {
            return false;
        }
        let start = self.data_offset as usize;
        let end = start + self.frames as usize * self.block_align as usize;
        let Some(data) = bytes.get(start..end) else {
            return false;
        };
        let width = self.encoding.bytes();
        data.chunks_exact(width)
            .zip(decoded)
            .all(|(raw, &want)| self.encoding.to_f32(raw).to_bits() == want.to_bits())
    }
}

/// The part of a take [`decode_wav_split`] leaves on disk: how to read
/// any of its frames, at the target rate, from the original file.
///
/// Every frame [`WavTail::read`] produces is bit-identical to the same
/// frame of [`decode_wav_native`] of the whole file: no resampling is a
/// straight conversion of the stored samples, and resampling evaluates
/// the very kernel the one-shot conversion does over the same input
/// frames ([`crate::resample::ResampleKernel`]). There is no decoder or
/// filter state to carry across the head/tail boundary.
#[derive(Clone)]
pub struct WavTail {
    layout: WavPcmLayout,
    /// 1 (mono) or 2, as the decode keeps them.
    channels: usize,
    kernel: Option<crate::resample::ResampleKernel>,
    /// Frames of the whole take at the target rate.
    frames: u64,
}

/// Reusable buffers for [`WavTail::read`], one per reading thread.
#[derive(Default)]
pub struct TailScratch {
    raw: Vec<u8>,
    window: Vec<f32>,
    row: Vec<f32>,
}

impl WavTail {
    /// 1 or 2 channels per output frame.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Frames of the whole take at the target rate.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The file's layout.
    pub fn layout(&self) -> &WavPcmLayout {
        &self.layout
    }

    /// True when the tail is converted to another rate as it is read.
    pub fn resamples(&self) -> bool {
        self.kernel.is_some()
    }

    /// Fill `out` (whole frames, [`channels`](Self::channels) per frame)
    /// with the take's frames from `out_start` on, read from `file` —
    /// the file the tail was split from. Frames past the end of the take
    /// are an error, as is a short read (the file shrank).
    pub fn read(
        &self,
        file: &std::fs::File,
        out_start: u64,
        out: &mut [f32],
        scratch: &mut TailScratch,
    ) -> std::io::Result<()> {
        let n = (out.len() / self.channels) as u64;
        if n == 0 {
            return Ok(());
        }
        if out_start + n > self.frames {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "read past the end of the take",
            ));
        }
        let (first, last) = match &self.kernel {
            None => (out_start, out_start + n - 1),
            Some(k) => {
                let (lo, hi) = k.input_span(out_start, out_start + n);
                let max = self.layout.frames as i64 - 1;
                (lo.clamp(0, max) as u64, hi.clamp(0, max) as u64)
            }
        };
        let align = self.layout.block_align as u64;
        let len = ((last - first + 1) * align) as usize;
        scratch.raw.resize(len, 0);
        read_exact_at(file, &mut scratch.raw, self.layout.data_offset + first * align)?;
        match &self.kernel {
            None => {
                scratch.window.clear();
                self.layout.convert(&scratch.raw, self.channels, &mut scratch.window);
                out[..scratch.window.len()].copy_from_slice(&scratch.window);
            }
            Some(k) => {
                scratch.window.clear();
                self.layout.convert(&scratch.raw, self.channels, &mut scratch.window);
                let frames = self.layout.frames;
                let window = &scratch.window;
                match self.channels {
                    1 => k.render::<1>(frames, window, first, out_start, out, &mut scratch.row),
                    _ => k.render::<2>(frames, window, first, out_start, out, &mut scratch.row),
                }
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn read_exact_at(file: &std::fs::File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, offset)
}

#[cfg(windows)]
fn read_exact_at(file: &std::fs::File, mut buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match file.seek_read(buf, offset) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "short read",
                ))
            }
            Ok(n) => {
                buf = &mut buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// A take decoded by [`decode_wav_split`]: its first frames in memory,
/// and — when it was long enough to split — how to read the rest.
pub struct SplitAudio {
    /// Interleaved, `channels` per frame: the whole take, or its head.
    pub samples: Vec<f32>,
    /// 1 or 2, as [`decode_wav_native`] keeps them.
    pub channels: usize,
    /// Frames of the whole take at the target rate.
    pub frames: usize,
    /// The frames past `samples`, on disk. `None`: `samples` is the
    /// whole take.
    pub tail: Option<WavTail>,
}

/// [`decode_wav_native`], keeping only the first `head_frames` frames in
/// memory when the take is longer than `head_frames + min_tail_frames`;
/// the rest is described by [`SplitAudio::tail`], to be read from the
/// file on demand.
///
/// The head is bit-identical to the same frames of the full decode, and
/// so is every frame the tail reads. A file is split only when its PCM
/// can be read straight from disk — plain integer or float PCM whose
/// conversion matches the decoder's output bit for bit (checked over the
/// whole file here). Anything else (another codec, an odd header, a
/// mismatch) comes back whole, exactly as `decode_wav_native` gives it.
/// `head_frames == 0` never splits.
pub fn decode_wav_split(
    data: Vec<u8>,
    target_sample_rate: f32,
    head_frames: usize,
    min_tail_frames: usize,
) -> Result<SplitAudio, WavDecodeError> {
    let layout = if head_frames > 0 {
        WavPcmLayout::parse(&data)
    } else {
        None
    };
    let bytes = std::sync::Arc::new(data);
    let cursor = Cursor::new(SharedBytes(bytes.clone()));
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("wav");
    let decoded = decode_source_to_interleaved(mss, hint, "WAV")?;
    let source_rate = decoded.sample_rate;
    let resample = (source_rate - target_sample_rate).abs() > 1.0;
    let kernel = if resample {
        crate::resample::ResampleKernel::for_rates(source_rate, target_sample_rate)
    } else {
        None
    };
    let in_frames = (decoded.samples.len() / decoded.channels) as u64;
    let out_frames = kernel.as_ref().map_or(in_frames, |k| k.out_len(in_frames));
    let channels = if decoded.channels == 1 { 1 } else { 2 };

    let streamable = layout.filter(|l| {
        out_frames > (head_frames + min_tail_frames) as u64
            && l.channels as usize == decoded.channels
            && l.sample_rate as f32 == source_rate
            && l.matches(&bytes, &decoded.samples)
    });
    drop(bytes);
    let Some(layout) = streamable else {
        // Whole, exactly as `decode_wav_native` builds it.
        let samples = if channels == 1 {
            if resample {
                linear_resample_mono(&decoded.samples, source_rate, target_sample_rate)
            } else {
                decoded.samples
            }
        } else {
            let stereo = if decoded.channels == 2 {
                decoded.samples
            } else {
                to_stereo_interleaved(&decoded.samples, decoded.channels)
            };
            if resample {
                linear_resample_stereo(&stereo, source_rate, target_sample_rate)
            } else {
                stereo
            }
        };
        let frames = samples.len() / channels;
        return Ok(SplitAudio {
            samples,
            channels,
            frames,
            tail: None,
        });
    };

    // Keep the first `channels` of every frame, then the head of that.
    let kept = if decoded.channels == channels {
        decoded.samples
    } else {
        let mut kept = Vec::with_capacity(in_frames as usize * channels);
        for frame in decoded.samples.chunks_exact(decoded.channels) {
            kept.extend_from_slice(&frame[..channels]);
        }
        kept
    };
    let head = match &kernel {
        None => {
            let mut kept = kept;
            kept.truncate(head_frames * channels);
            kept.shrink_to_fit();
            kept
        }
        Some(k) => {
            let mut head = vec![0.0f32; head_frames * channels];
            let mut row = Vec::new();
            match channels {
                1 => k.render::<1>(in_frames, &kept, 0, 0, &mut head, &mut row),
                _ => k.render::<2>(in_frames, &kept, 0, 0, &mut head, &mut row),
            }
            head
        }
    };
    Ok(SplitAudio {
        samples: head,
        channels,
        frames: out_frames as usize,
        tail: Some(WavTail {
            layout,
            channels,
            kernel,
            frames: out_frames,
        }),
    })
}

/// File bytes the decoder reads while the caller keeps them too (to check
/// the decode against the raw PCM).
struct SharedBytes(std::sync::Arc<Vec<u8>>);

impl AsRef<[u8]> for SharedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
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
        source_rate,
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
