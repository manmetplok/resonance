//! Memory-mapping a 32-bit float stereo WAV file and locating its PCM
//! data chunk (ba todo #1260).
//!
//! Clips stream their audio straight out of the page cache: the engine
//! maps the file once, off the audio thread, and the mixer then reads
//! `&[f32]` directly out of the mapping with no copy and no decode. That
//! buys zero-copy playback at the price of one `unsafe` — kept here, in
//! [`map_wav_file`], and nowhere else.
//!
//! The chunk walk ([`locate_wav_float_data`]) is a pure function over a
//! byte slice with no I/O of its own, so every malformed-header case is
//! directly testable (`tests/io/wav_chunk_parse.rs`).

use std::path::Path;

use thiserror::Error;

use crate::types::{EngineError, EngineErrorKind};

/// Failure parsing a RIFF/WAVE header into its `fmt`/`data` chunks, or
/// validating that the format is the stereo 32-bit float this crate can
/// map. Pure (no I/O); message text matches the historical `format!()`
/// strings.
#[derive(Debug, Error)]
pub enum WavParseError {
    #[error("file too short")]
    TooShort,
    #[error("missing RIFF header")]
    MissingRiff,
    #[error("not a WAVE file")]
    NotWave,
    #[error("chunk {id_debug} overruns file")]
    ChunkOverrun { id_debug: String },
    #[error("data chunk before fmt chunk")]
    DataBeforeFmt,
    #[error("no data chunk found")]
    NoDataChunk,
    #[error("fmt chunk too small")]
    FmtTooSmall,
    #[error("extensible fmt chunk too small")]
    ExtensibleFmtTooSmall,
    #[error("unsupported format code {code} (expected 3, IEEE float)")]
    UnsupportedFormatCode { code: u16 },
    #[error("expected stereo, got {channels} channels")]
    ExpectedStereo { channels: u16 },
    #[error("expected 32-bit float, got {bits} bits")]
    ExpectedFloat32 { bits: u16 },
    #[error("fmt chunk declares zero sample rate")]
    ZeroSampleRate,
    #[error("data chunk length {data_len_bytes} not a multiple of stereo f32 frames")]
    NotWholeFrames { data_len_bytes: usize },
}

/// Failure opening and memory-mapping a WAV file for zero-copy playback:
/// the filesystem open, the `mmap`, or [`WavParseError`] on the mapped
/// bytes. Message text matches the historical `format!()` strings.
#[derive(Debug, Error)]
pub enum WavIoError {
    #[error("open wav {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("mmap {path}: {source}")]
    Mmap {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parse wav {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: WavParseError,
    },
}

impl From<WavIoError> for EngineError {
    fn from(e: WavIoError) -> Self {
        let kind = match &e {
            WavIoError::Open { .. } | WavIoError::Mmap { .. } => EngineErrorKind::Io,
            WavIoError::Parse { .. } => EngineErrorKind::Unsupported,
        };
        EngineError::new(kind, e.to_string())
    }
}

/// A mapped WAV file, resolved down to its PCM payload.
pub struct MappedWav {
    /// The whole file, mapped read-only.
    pub mmap: memmap2::Mmap,
    /// Byte offset from the start of the mapping at which interleaved
    /// f32 samples begin.
    pub data_offset_bytes: usize,
    /// Number of stereo frames in the data chunk.
    pub frame_count: u64,
    /// Sample rate declared by the fmt chunk.
    pub sample_rate: u32,
}

/// Where a WAV's PCM payload lives, as reported by
/// [`locate_wav_float_data`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavDataChunk {
    /// Byte offset of the first sample from the start of the file.
    pub data_offset_bytes: usize,
    /// Length of the data chunk in bytes.
    pub data_len_bytes: usize,
    /// Sample rate declared by the fmt chunk that preceded it.
    pub sample_rate: u32,
}

impl WavDataChunk {
    /// Stereo f32 frames in the data chunk, or an error when the chunk
    /// length is not a whole number of them.
    pub fn stereo_frame_count(&self) -> Result<u64, WavParseError> {
        const STEREO_FRAME_BYTES: usize = 2 * std::mem::size_of::<f32>();
        if !self.data_len_bytes.is_multiple_of(STEREO_FRAME_BYTES) {
            return Err(WavParseError::NotWholeFrames {
                data_len_bytes: self.data_len_bytes,
            });
        }
        Ok((self.data_len_bytes / STEREO_FRAME_BYTES) as u64)
    }
}

/// Open a 32-bit-float stereo WAV file, memory-map it, and resolve the
/// offset / length / rate of its PCM data chunk. Also pre-touches every
/// page of that chunk so the first mixer access doesn't take a major page
/// fault on the realtime audio thread.
///
/// This is the crate's only file mapping: every caller that wants clip
/// audio off disk goes through here, so the safety argument below has to
/// hold in exactly one place.
pub fn map_wav_file(path: &Path) -> Result<MappedWav, WavIoError> {
    let file = std::fs::File::open(path).map_err(|e| WavIoError::Open {
        path: path.display().to_string(),
        source: e,
    })?;
    // SAFETY: `Mmap::map` is unsafe because the kernel can serve a
    // shared mapping whose backing file is truncated or written by
    // another process while we hold a `&[u8]` over it — a UB hazard
    // in general. We control the lifecycle here: vocal WAVs come from
    // our own renderer, sample-imported WAVs are user-owned read-only
    // files, and the `unlink` paths in `vocal_render::tear_down_old_*`
    // run only after the mixer has dropped the `Mapped` ClipSource
    // referring to them. There is no writer to this file while the
    // mapping is live. Outside that contract — e.g. another process
    // truncating an imported file — the worst case is the mixer
    // reading a SIGBUS-poisoned page; that's a failure mode we accept
    // in exchange for zero-copy audio streaming.
    let mmap = unsafe { memmap2::Mmap::map(&file) }.map_err(|e| WavIoError::Mmap {
        path: path.display().to_string(),
        source: e,
    })?;

    let chunk = locate_wav_float_data(&mmap).map_err(|e| WavIoError::Parse {
        path: path.display().to_string(),
        source: e,
    })?;
    let frame_count = chunk.stereo_frame_count().map_err(|e| WavIoError::Parse {
        path: path.display().to_string(),
        source: e,
    })?;

    // Pre-touch: read one byte per 4 KiB page across the data
    // chunk so that the first mixer access doesn't trigger
    // major page faults on the realtime audio thread.
    pre_touch(&mmap[chunk.data_offset_bytes..chunk.data_offset_bytes + chunk.data_len_bytes]);

    Ok(MappedWav {
        mmap,
        data_offset_bytes: chunk.data_offset_bytes,
        frame_count,
        sample_rate: chunk.sample_rate,
    })
}

/// Parse a minimal RIFF/WAVE header and return the byte offset and
/// length of the PCM `data` chunk plus the fmt-chunk sample rate,
/// verifying that the format chunk declares 32-bit IEEE float stereo.
/// Does not depend on `hound`.
pub fn locate_wav_float_data(bytes: &[u8]) -> Result<WavDataChunk, WavParseError> {
    if bytes.len() < 12 {
        return Err(WavParseError::TooShort);
    }
    if &bytes[0..4] != b"RIFF" {
        return Err(WavParseError::MissingRiff);
    }
    if &bytes[8..12] != b"WAVE" {
        return Err(WavParseError::NotWave);
    }

    let mut cursor = 12usize;
    let mut fmt_sample_rate: Option<u32> = None;
    while cursor + 8 <= bytes.len() {
        let id = &bytes[cursor..cursor + 4];
        let size = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        let chunk_start = cursor + 8;
        let chunk_end = chunk_start + size;
        if chunk_end > bytes.len() {
            return Err(WavParseError::ChunkOverrun {
                id_debug: format!("{:?}", std::str::from_utf8(id)),
            });
        }

        if id == b"fmt " {
            fmt_sample_rate = Some(parse_float_fmt_chunk(&bytes[chunk_start..chunk_end])?);
        } else if id == b"data" {
            let Some(sample_rate) = fmt_sample_rate else {
                return Err(WavParseError::DataBeforeFmt);
            };
            return Ok(WavDataChunk {
                data_offset_bytes: chunk_start,
                data_len_bytes: size,
                sample_rate,
            });
        }

        // RIFF chunks are word-aligned: an odd size is padded.
        cursor = chunk_end + (size & 1);
    }
    Err(WavParseError::NoDataChunk)
}

/// Validate one `fmt ` chunk body and return its sample rate. The engine
/// only maps what its mixer can read straight out of the page cache:
/// stereo, 32-bit IEEE float.
fn parse_float_fmt_chunk(chunk: &[u8]) -> Result<u32, WavParseError> {
    let size = chunk.len();
    if size < 16 {
        return Err(WavParseError::FmtTooSmall);
    }
    let format = u16::from_le_bytes(chunk[0..2].try_into().unwrap());
    let channels = u16::from_le_bytes(chunk[2..4].try_into().unwrap());
    let sample_rate = u32::from_le_bytes(chunk[4..8].try_into().unwrap());
    let bits_per_sample = u16::from_le_bytes(chunk[14..16].try_into().unwrap());
    // `hound` writes float WAVs using WAVE_FORMAT_EXTENSIBLE
    // (0xFFFE) with a SubFormat GUID. The first two bytes
    // of that GUID carry the real format code, so we
    // inspect them instead of the outer format tag.
    const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
    const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
    let effective_format = if format == WAVE_FORMAT_EXTENSIBLE {
        if size < 40 {
            return Err(WavParseError::ExtensibleFmtTooSmall);
        }
        u16::from_le_bytes(chunk[24..26].try_into().unwrap())
    } else {
        format
    };
    if effective_format != WAVE_FORMAT_IEEE_FLOAT {
        return Err(WavParseError::UnsupportedFormatCode {
            code: effective_format,
        });
    }
    if channels != 2 {
        return Err(WavParseError::ExpectedStereo { channels });
    }
    if bits_per_sample != 32 {
        return Err(WavParseError::ExpectedFloat32 {
            bits: bits_per_sample,
        });
    }
    if sample_rate == 0 {
        return Err(WavParseError::ZeroSampleRate);
    }
    Ok(sample_rate)
}

/// Fault in every page of `bytes` by reading one byte per 4 KiB.
///
/// The 4 KiB step is deliberate, including on systems with transparent
/// huge pages enabled. The THP sysfs knobs
/// (`/sys/kernel/mm/transparent_hugepage/enabled` / `shmem_enabled`)
/// govern anonymous and tmpfs/shmem memory; this is a private read-only
/// *file-backed* mapping, which the page cache populates with base
/// pages (or filesystem-chosen large folios, independent of those
/// knobs). Stepping by `hpage_pmd_size` whenever the knob reads
/// `[always]` would therefore skip 511 of every 512 pages in the common
/// case where the mapping is in fact 4 KiB-paged — reintroducing major
/// faults on the realtime mixer thread, the exact failure this function
/// exists to prevent. When the kernel does back a region with a larger
/// folio, the surplus reads are one cache-hot load per 4 KiB (no
/// fault), which is noise next to the mmap + WAV parse around this
/// call. Discovering the actual folio size would mean parsing
/// `/proc/self/smaps` per mapping; not worth it for that noise.
fn pre_touch(bytes: &[u8]) {
    let page = 4096usize;
    let mut i = 0usize;
    let mut acc: u8 = 0;
    while i < bytes.len() {
        acc ^= bytes[i];
        i += page;
    }
    // Prevent the read loop from being optimised away.
    std::hint::black_box(acc);
}
