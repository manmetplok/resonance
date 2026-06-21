//! Pluggable encoder sinks for the offline export pipeline (doc #196).
//!
//! The render loop produces interleaved stereo `f32` frames; an
//! [`EncoderSink`] consumes them and writes the encoded file. Today two
//! sinks exist — [`WavSink`] (16/24-bit PCM or 32-bit float) and
//! [`FlacSink`] (lossless 16/24-bit via the pure-Rust `flacenc`). The
//! 32-bit-float WAV path is byte-for-byte identical to the legacy hound
//! tail it replaces.
//!
//! Sinks are constructed via [`build_sink`], which is the single place
//! that decides whether a format's encoder is available. Formats whose
//! encoder is not yet wired (MP3/Opus land in #651) return
//! [`EncoderError::Unavailable`] *before any file is created*, so the
//! caller can surface `ExportErrorKind::EncoderUnavailable` without
//! leaving a partial file behind.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use crate::types::{BitDepth, ExportFormat, ExportMetadata, FlacLevel};

/// Why an encoder sink could not be built, written or finalized.
#[derive(Debug)]
pub(super) enum EncoderError {
    /// The selected format's encoder is unavailable (not compiled in / not
    /// yet implemented). Returned by [`build_sink`] *before* any file is
    /// created. Maps to `ExportErrorKind::EncoderUnavailable`.
    Unavailable(String),
    /// Filesystem / encoder error while creating, writing or finalizing the
    /// output file. Maps to `ExportErrorKind::Io`.
    Io(String),
}

impl EncoderError {
    pub(super) fn message(&self) -> &str {
        match self {
            EncoderError::Unavailable(m) | EncoderError::Io(m) => m,
        }
    }
}

/// A pluggable encoder for the offline export pipeline. Frames arrive as
/// interleaved stereo `f32` (length always a multiple of 2) at the
/// sink's output sample rate — any export-time resampling happens before
/// the sink (see [`super::resample`]).
pub(super) trait EncoderSink {
    /// Append `frames` (interleaved stereo f32) to the output.
    fn write_frames(&mut self, frames: &[f32]) -> Result<(), EncoderError>;
    /// Flush, finalize and close the file, embedding `meta`. Returns the
    /// encoded file size in bytes.
    fn finalize(self: Box<Self>, meta: &ExportMetadata) -> Result<u64, EncoderError>;
}

/// Build the encoder sink for `format`, writing to `path` at the already
/// resolved output `sample_rate`. Formats without an available encoder
/// return [`EncoderError::Unavailable`] and create no file.
pub(super) fn build_sink(
    format: &ExportFormat,
    sample_rate: u32,
    path: &Path,
) -> Result<Box<dyn EncoderSink>, EncoderError> {
    match *format {
        ExportFormat::Wav { bit_depth, .. } => {
            Ok(Box::new(WavSink::create(path, sample_rate, bit_depth)?))
        }
        ExportFormat::Flac {
            bit_depth,
            compression,
            ..
        } => Ok(Box::new(FlacSink::new(
            path,
            sample_rate,
            bit_depth,
            compression,
        )?)),
        ExportFormat::Mp3 { .. } => Err(EncoderError::Unavailable(
            "MP3 export is not available in this build".into(),
        )),
        ExportFormat::Opus { .. } => Err(EncoderError::Unavailable(
            "Opus export is not available in this build".into(),
        )),
    }
}

// --- Dither -----------------------------------------------------------------

/// Deterministic TPDF dither generator for integer quantization. Seeded
/// from a fixed constant so a given mix re-exports bit-for-bit, which
/// keeps round-trip tests stable. The triangular distribution (sum of two
/// independent uniforms) decorrelates quantization error from the signal.
struct Dither(u64);

impl Dither {
    fn new() -> Self {
        // Arbitrary non-zero seed; any fixed value gives reproducible noise.
        Dither(0x9E37_79B9_7F4A_7C15)
    }

    /// One uniform sample in `[0, 1)` from a 64-bit LCG (Knuth's MMIX
    /// constants), taking the high bits for better distribution.
    fn next_uniform(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / ((1u32 << 24) as f32)
    }

    /// One triangular dither sample in `(-1, 1)` LSBs.
    fn next_tpdf(&mut self) -> f32 {
        self.next_uniform() - self.next_uniform()
    }
}

/// Quantize one `f32` sample to an integer of `bits` bit depth with TPDF
/// `dither` (in LSBs). Full scale maps `-1.0 -> -2^(bits-1)` and
/// `+1.0 -> 2^(bits-1)-1`, clamped to the representable range.
#[inline]
fn quantize(sample: f32, bits: u32, dither: f32) -> i32 {
    let scale = (1i64 << (bits - 1)) as f32;
    let max = (1i64 << (bits - 1)) - 1;
    let min = -(1i64 << (bits - 1));
    let v = (sample.clamp(-1.0, 1.0) * scale + dither).round() as i64;
    v.clamp(min, max) as i32
}

// --- WAV --------------------------------------------------------------------

/// WAV sink backed by `hound`. `F32` writes 32-bit IEEE float byte-for-byte
/// identical to the legacy bounce; `I16`/`I24` write dithered integer PCM.
pub(super) struct WavSink {
    writer: hound::WavWriter<BufWriter<File>>,
    path: PathBuf,
    bit_depth: BitDepth,
    dither: Dither,
}

impl WavSink {
    fn create(path: &Path, sample_rate: u32, bit_depth: BitDepth) -> Result<Self, EncoderError> {
        let bits_per_sample = match bit_depth {
            BitDepth::I16 => 16,
            BitDepth::I24 => 24,
            BitDepth::F32 => 32,
        };
        let sample_format = match bit_depth {
            BitDepth::F32 => hound::SampleFormat::Float,
            _ => hound::SampleFormat::Int,
        };
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate,
            bits_per_sample,
            sample_format,
        };
        let writer = hound::WavWriter::create(path, spec)
            .map_err(|e| EncoderError::Io(format!("Failed to create WAV file: {e}")))?;
        Ok(WavSink {
            writer,
            path: path.to_path_buf(),
            bit_depth,
            dither: Dither::new(),
        })
    }
}

impl EncoderSink for WavSink {
    fn write_frames(&mut self, frames: &[f32]) -> Result<(), EncoderError> {
        let map_err = |e: hound::Error| EncoderError::Io(format!("WAV write error: {e}"));
        match self.bit_depth {
            BitDepth::F32 => {
                // Byte-for-byte identical to the legacy hound tail.
                for &s in frames {
                    self.writer.write_sample(s).map_err(map_err)?;
                }
            }
            BitDepth::I16 => {
                for &s in frames {
                    let q = quantize(s, 16, self.dither.next_tpdf()) as i16;
                    self.writer.write_sample(q).map_err(map_err)?;
                }
            }
            BitDepth::I24 => {
                for &s in frames {
                    let q = quantize(s, 24, self.dither.next_tpdf());
                    // hound emits a 24-bit sample when the spec is 24-bit.
                    self.writer.write_sample(q).map_err(map_err)?;
                }
            }
        }
        Ok(())
    }

    fn finalize(self: Box<Self>, _meta: &ExportMetadata) -> Result<u64, EncoderError> {
        let path = self.path;
        self.writer
            .finalize()
            .map_err(|e| EncoderError::Io(format!("WAV finalize error: {e}")))?;
        file_size(&path)
    }
}

// --- FLAC -------------------------------------------------------------------

/// FLAC sink backed by the pure-Rust `flacenc`. `flacenc` encodes from a
/// fully materialized sample buffer, so frames are quantized and buffered
/// here and the bitstream is produced in [`finalize`](EncoderSink::finalize) —
/// which also means a cancelled export never leaves a partial `.flac` on
/// disk (nothing is written until finalize).
pub(super) struct FlacSink {
    path: PathBuf,
    sample_rate: u32,
    bits: u32,
    block_size: usize,
    /// Interleaved stereo integer PCM accumulated across `write_frames`.
    samples: Vec<i32>,
    dither: Dither,
}

impl FlacSink {
    fn new(
        path: &Path,
        sample_rate: u32,
        bit_depth: BitDepth,
        compression: FlacLevel,
    ) -> Result<Self, EncoderError> {
        // FLAC is integer-only; `F32` is not a valid FLAC depth and the UI
        // never offers it, but guard so a hand-built spec can't panic later.
        let bits = match bit_depth {
            BitDepth::I16 => 16,
            BitDepth::I24 => 24,
            BitDepth::F32 => {
                return Err(EncoderError::Unavailable(
                    "FLAC supports only 16- or 24-bit depth".into(),
                ))
            }
        };
        Ok(FlacSink {
            path: path.to_path_buf(),
            sample_rate,
            bits,
            block_size: flac_block_size(compression),
            samples: Vec::new(),
            dither: Dither::new(),
        })
    }
}

/// Rewrite the encoded stream's STREAMINFO so it reports `real_frames`
/// total samples instead of the padded count, and clear the now-stale MD5
/// signature (an all-zero MD5 is the spec's "not computed" sentinel, so no
/// decoder fails verification against the trimmed audio).
///
/// STREAMINFO is the first metadata block: 4-byte `fLaC` marker + 4-byte
/// block header, then the 34-byte body. `total_samples` is the low 36 bits
/// of the 8 bytes at body offset 10 (file offset 18); the MD5 is the final
/// 16 bytes (file offset 26). A guard keeps a truncated stream from
/// panicking, though a successful encode always yields a full STREAMINFO.
fn patch_streaminfo_total_samples(bytes: &mut [u8], real_frames: u64) {
    const TOTAL_SAMPLES_OFFSET: usize = 18;
    const MD5_OFFSET: usize = 26;
    const MD5_END: usize = 42;
    if bytes.len() < MD5_END {
        return;
    }
    let packed = u64::from_be_bytes(bytes[TOTAL_SAMPLES_OFFSET..MD5_OFFSET].try_into().unwrap());
    let patched = (packed & !0xF_FFFF_FFFF) | (real_frames & 0xF_FFFF_FFFF);
    bytes[TOTAL_SAMPLES_OFFSET..MD5_OFFSET].copy_from_slice(&patched.to_be_bytes());
    bytes[MD5_OFFSET..MD5_END].fill(0);
}

/// Map the user-facing [`FlacLevel`] onto a `flacenc` block size. Larger
/// blocks let the encoder model longer-range correlation for a smaller
/// file at the cost of encode time; the decoded audio is identical for
/// every level (FLAC is lossless), so this only trades size against speed.
fn flac_block_size(level: FlacLevel) -> usize {
    match level {
        FlacLevel::Fast => 1024,
        FlacLevel::Default => 4096,
        FlacLevel::Max => 8192,
    }
}

impl EncoderSink for FlacSink {
    fn write_frames(&mut self, frames: &[f32]) -> Result<(), EncoderError> {
        self.samples.reserve(frames.len());
        for &s in frames {
            self.samples.push(quantize(s, self.bits, self.dither.next_tpdf()));
        }
        Ok(())
    }

    fn finalize(self: Box<Self>, _meta: &ExportMetadata) -> Result<u64, EncoderError> {
        use flacenc::component::BitRepr;
        use flacenc::error::Verify;

        let mut config = flacenc::config::Encoder::default();
        config.block_size = self.block_size;
        // Single-threaded encode: the multi-threaded path emits frames with
        // a per-worker frame counter, which strict FLAC readers (symphonia)
        // reject as a non-monotonic frame number. Export runs off the audio
        // thread already, so the serial encoder costs nothing user-visible.
        config.multithread = false;
        let config = config
            .into_verified()
            .map_err(|e| EncoderError::Io(format!("FLAC config error: {e:?}")))?;

        // symphonia 0.6 (the app's own decoder) cannot read a fixed-block
        // FLAC whose final frame is shorter than the block size — probing
        // such a file fails outright. Pad the tail with silence so every
        // frame is a full block, then restore the true sample count in
        // STREAMINFO below so decoders return exactly the rendered audio and
        // drop the padding. The file stays a valid, spec-compliant FLAC.
        let real_frames = (self.samples.len() / 2) as u64;
        let mut samples = self.samples;
        let padded_frames = (samples.len() / 2).div_ceil(self.block_size) * self.block_size;
        let padded = padded_frames * 2 != samples.len();
        samples.resize(padded_frames * 2, 0);

        let source = flacenc::source::MemSource::from_samples(
            &samples,
            2,
            self.bits as usize,
            self.sample_rate as usize,
        );
        let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
            .map_err(|e| EncoderError::Io(format!("FLAC encode error: {e:?}")))?;

        let mut sink = flacenc::bitsink::ByteSink::new();
        stream
            .write(&mut sink)
            .map_err(|e| EncoderError::Io(format!("FLAC write error: {e:?}")))?;
        let mut bytes = sink.as_slice().to_vec();

        if padded {
            patch_streaminfo_total_samples(&mut bytes, real_frames);
        }

        std::fs::write(&self.path, &bytes)
            .map_err(|e| EncoderError::Io(format!("Failed to write FLAC file: {e}")))?;
        Ok(bytes.len() as u64)
    }
}

/// Stat the finished file for the byte count reported in `ExportComplete`.
fn file_size(path: &Path) -> Result<u64, EncoderError> {
    std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| EncoderError::Io(format!("Failed to stat output file: {e}")))
}
