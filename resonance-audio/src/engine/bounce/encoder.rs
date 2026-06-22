//! Pluggable encoder sinks for the offline export pipeline (doc #196).
//!
//! The render loop produces interleaved stereo `f32` frames; an
//! [`EncoderSink`] consumes them and writes the encoded file. Today three
//! sinks exist — [`WavSink`] (16/24-bit PCM or 32-bit float), [`FlacSink`]
//! (lossless 16/24-bit via the pure-Rust `flacenc`) and [`Mp3Sink`] (lossy
//! CBR/VBR via `libmp3lame`, behind the `mp3` feature). The 32-bit-float
//! WAV path is byte-for-byte identical to the legacy hound tail it
//! replaces.
//!
//! Sinks are constructed via [`build_sink`], which is the single place
//! that decides whether a format's encoder is available. Formats whose
//! encoder is not compiled in (MP3 without the `mp3` feature) or not yet
//! wired (Opus lands in #651) return [`EncoderError::Unavailable`] *before
//! any file is created*, so the caller can surface
//! `ExportErrorKind::EncoderUnavailable` without leaving a partial file
//! behind.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

#[cfg(feature = "mp3")]
use crate::types::Mp3Rate;
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
        ExportFormat::Mp3 { mode, bitrate_kbps } => {
            #[cfg(feature = "mp3")]
            {
                Ok(Box::new(Mp3Sink::new(path, sample_rate, mode, bitrate_kbps)?))
            }
            #[cfg(not(feature = "mp3"))]
            {
                let _ = (mode, bitrate_kbps);
                Err(EncoderError::Unavailable(
                    "MP3 export is not available in this build (compile with the `mp3` feature)"
                        .into(),
                ))
            }
        }
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

// --- MP3 --------------------------------------------------------------------

/// MP3 sink backed by `libmp3lame` (built from bundled C source by the
/// `mp3lame-encoder` crate, behind the `mp3` feature). Interleaved stereo
/// `f32` frames feed LAME's IEEE-float input directly (full-scale ±1.0, no
/// intermediate quantization — LAME applies its own psychoacoustic
/// quantization). The whole MPEG stream is buffered in RAM and written once
/// in [`finalize`], so — like [`FlacSink`] — a cancelled export never
/// leaves a partial `.mp3` on disk.
///
/// `Cbr` holds a constant bitrate; `Vbr` targets a quality level (mapped
/// from `bitrate_kbps`) and lets the bitrate float. A LAME "Info" (CBR) /
/// "Xing" (VBR) header frame is prepended at finalize so players report the
/// correct duration and honour encoder delay/padding for gapless playback.
#[cfg(feature = "mp3")]
pub(super) struct Mp3Sink {
    path: PathBuf,
    encoder: mp3lame_encoder::Encoder,
    /// Encoded MPEG audio frames accumulated across `write_frames`. The
    /// Info/Xing header is spliced in front in `finalize`.
    out: Vec<u8>,
}

#[cfg(feature = "mp3")]
impl Mp3Sink {
    fn new(
        path: &Path,
        sample_rate: u32,
        mode: Mp3Rate,
        bitrate_kbps: u32,
    ) -> Result<Self, EncoderError> {
        use mp3lame_encoder::{Builder, Quality, VbrMode};

        let io = |e: mp3lame_encoder::BuildError| {
            EncoderError::Io(format!("MP3 encoder setup error: {e:?}"))
        };
        let mut builder = Builder::new()
            .ok_or_else(|| EncoderError::Io("Failed to allocate LAME encoder".into()))?;
        builder.set_num_channels(2).map_err(io)?;
        builder.set_sample_rate(sample_rate).map_err(io)?;
        // Best LAME analysis quality (`-q 0`): export is offline, so the
        // slowest / highest-quality setting costs nothing user-visible.
        builder.set_quality(Quality::Best).map_err(io)?;
        match mode {
            Mp3Rate::Cbr => {
                builder.set_brate(cbr_bitrate(bitrate_kbps)).map_err(io)?;
            }
            Mp3Rate::Vbr => {
                builder.set_vbr_mode(VbrMode::Mtrh).map_err(io)?;
                builder.set_vbr_quality(vbr_quality(bitrate_kbps)).map_err(io)?;
            }
        }
        // Emit a LAME Info (CBR) / Xing (VBR) header frame so players read
        // the correct duration and drop encoder delay/padding for gapless
        // playback. It is spliced in front of the audio in `finalize`.
        builder.set_to_write_vbr_tag(true).map_err(io)?;
        let encoder = builder.build().map_err(io)?;
        Ok(Mp3Sink {
            path: path.to_path_buf(),
            encoder,
            out: Vec::new(),
        })
    }
}

#[cfg(feature = "mp3")]
impl EncoderSink for Mp3Sink {
    fn write_frames(&mut self, frames: &[f32]) -> Result<(), EncoderError> {
        use mp3lame_encoder::{max_required_buffer_size, InterleavedPcm};
        // `encode_to_vec` writes into spare capacity only, so reserve LAME's
        // worst-case output size for this chunk up front (per-channel sample
        // count = interleaved length / 2).
        self.out
            .reserve(max_required_buffer_size(frames.len() / 2));
        self.encoder
            .encode_to_vec(InterleavedPcm(frames), &mut self.out)
            .map_err(|e| EncoderError::Io(format!("MP3 encode error: {e:?}")))?;
        Ok(())
    }

    fn finalize(mut self: Box<Self>, _meta: &ExportMetadata) -> Result<u64, EncoderError> {
        use mp3lame_encoder::FlushGap;

        // Flush LAME's internal buffers (final partial frame + padding).
        // 7200 bytes is one max-size MPEG frame — the most a flush emits.
        self.out.reserve(7200);
        self.encoder
            .flush_to_vec::<FlushGap>(&mut self.out)
            .map_err(|e| EncoderError::Io(format!("MP3 flush error: {e:?}")))?;

        // Prepend the LAME Info/Xing header frame so it is the first frame
        // of the stream. No ID3v2 tag is written, so the splice boundary is
        // the start of the file (see `mp3lame_encoder`'s documented write
        // order: id3v2 tag, then VBR tag, then audio).
        let lame_tag_size = self.encoder.lame_tag_size();
        let mut file = Vec::with_capacity(lame_tag_size + self.out.len());
        if lame_tag_size > 0 {
            let mut tag = Vec::with_capacity(lame_tag_size);
            self.encoder.lame_tag_encode_to_vec(&mut tag);
            file.extend_from_slice(&tag);
        }
        file.extend_from_slice(&self.out);

        std::fs::write(&self.path, &file)
            .map_err(|e| EncoderError::Io(format!("Failed to write MP3 file: {e}")))?;
        Ok(file.len() as u64)
    }
}

/// Snap a requested kbps to the nearest libmp3lame constant bitrate. The
/// export UI offers 128/192/256/320; any other value picks the closest
/// supported rate (LAME only accepts a fixed set), defaulting to 192.
#[cfg(feature = "mp3")]
fn cbr_bitrate(kbps: u32) -> mp3lame_encoder::Bitrate {
    use mp3lame_encoder::Bitrate::*;
    const RATES: &[(u32, mp3lame_encoder::Bitrate)] = &[
        (8, Kbps8),
        (16, Kbps16),
        (24, Kbps24),
        (32, Kbps32),
        (40, Kbps40),
        (48, Kbps48),
        (64, Kbps64),
        (80, Kbps80),
        (96, Kbps96),
        (112, Kbps112),
        (128, Kbps128),
        (160, Kbps160),
        (192, Kbps192),
        (224, Kbps224),
        (256, Kbps256),
        (320, Kbps320),
    ];
    RATES
        .iter()
        .min_by_key(|(r, _)| (*r as i64 - kbps as i64).unsigned_abs())
        .map(|&(_, b)| b)
        .unwrap_or(Kbps192)
}

/// Map a target bitrate to a libmp3lame VBR quality (`-V`, 0 = highest
/// quality / largest file, 9 = lowest). Loosely follows the standard LAME
/// presets (V0 ≈ 245 kbps, V2 ≈ 190, V3 ≈ 175, V4 ≈ 165).
#[cfg(feature = "mp3")]
fn vbr_quality(kbps: u32) -> mp3lame_encoder::Quality {
    use mp3lame_encoder::Quality::*;
    match kbps {
        k if k >= 256 => Best,     // V0
        k if k >= 192 => NearBest, // V2
        k if k >= 160 => VeryNice, // V3
        k if k >= 128 => Nice,     // V4
        _ => Good,                 // V5
    }
}

/// Stat the finished file for the byte count reported in `ExportComplete`.
fn file_size(path: &Path) -> Result<u64, EncoderError> {
    std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| EncoderError::Io(format!("Failed to stat output file: {e}")))
}
