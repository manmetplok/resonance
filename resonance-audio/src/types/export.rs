//! Offline-export job model: format, loudness normalization and
//! metadata. Generalizes the WAV-only bounce into a format-agnostic,
//! optionally loudness-normalized export spec shared by the full-mix
//! bounce and (later) stem export. See doc #196.
//!
//! These types are the engine command/event boundary only — the
//! encoder sinks, resampling and two-pass normalization land in the
//! follow-up todos (#650–#655). They derive serde so the app can persist
//! the last-used export preset and reuse a single spec across commands.

use serde::{Deserialize, Serialize};

/// Sample bit depth / format for the encoded file. `I16`/`I24` are
/// integer PCM; `F32` is 32-bit IEEE float. FLAC accepts only the
/// integer depths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BitDepth {
    I16,
    I24,
    F32,
}

/// FLAC compression effort. Maps to libFLAC compression levels:
/// `Fast` = 0, `Default` = 5, `Max` = 8. Higher levels trade encode
/// time for a smaller file; the decoded audio is identical (FLAC is
/// lossless).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlacLevel {
    Fast,
    Default,
    Max,
}

/// MP3 rate-control strategy. `Cbr` holds a constant bitrate; `Vbr`
/// targets a quality level and lets the bitrate float. The kbps figure
/// lives alongside in [`ExportFormat::Mp3::bitrate_kbps`] (the nominal
/// bitrate for CBR, the target for VBR).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mp3Rate {
    Cbr,
    Vbr,
}

/// Opus encoder application hint. `Music` optimizes for general audio;
/// `Voice` biases toward speech intelligibility at low bitrates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpusOptimize {
    Music,
    Voice,
}

/// Output container + codec and its quality parameters.
///
/// `sample_rate: None` keeps the engine sample rate; `Some(sr)` resamples
/// on export (resampling lands with the encoder-sink todo). Opus always
/// runs at 48 kHz internally regardless of this field.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ExportFormat {
    Wav {
        bit_depth: BitDepth,
        sample_rate: Option<u32>,
    },
    Flac {
        /// FLAC supports only integer depths (`I16` / `I24`).
        bit_depth: BitDepth,
        sample_rate: Option<u32>,
        compression: FlacLevel,
    },
    Mp3 {
        mode: Mp3Rate,
        bitrate_kbps: u32,
    },
    Opus {
        bitrate_kbps: u32,
        optimize: OpusOptimize,
    },
}

impl ExportFormat {
    /// The default 32-bit-float WAV format — byte-for-byte the output of
    /// the legacy `BounceToWav` path (stereo f32 at the engine rate).
    pub const fn default_wav() -> Self {
        ExportFormat::Wav {
            bit_depth: BitDepth::F32,
            sample_rate: None,
        }
    }
}

/// What a [`NormalizeSpec`] targets. `IntegratedLufs` measures
/// integrated program loudness (two-pass); `TruePeak` targets an
/// oversampled true-peak level (single-pass gain trim).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NormalizeMode {
    IntegratedLufs,
    TruePeak,
}

/// Optional loudness-normalization stage applied in front of the encoder
/// sink. When `enabled`, the export measures the rendered mix, computes a
/// gain trim toward `target_db` (LUFS or dBTP per `mode`), then brick-wall
/// limits true peaks to `ceiling_dbtp` before encoding.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NormalizeSpec {
    pub enabled: bool,
    pub mode: NormalizeMode,
    /// Target loudness: LUFS for `IntegratedLufs`, dBTP for `TruePeak`.
    pub target_db: f32,
    /// True-peak ceiling for the post-trim limiter, in dBTP.
    pub ceiling_dbtp: f32,
}

impl Default for NormalizeSpec {
    fn default() -> Self {
        // Disabled by default; values match the UI's streaming preset
        // (−14 LUFS integrated, −1 dBTP ceiling) so an enable-toggle
        // lands on sensible numbers.
        NormalizeSpec {
            enabled: false,
            mode: NormalizeMode::IntegratedLufs,
            target_db: -14.0,
            ceiling_dbtp: -1.0,
        }
    }
}

/// Container-embedded tags. Every field is optional; empty fields are
/// omitted from the file's metadata block.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<u16>,
}

/// Full offline-export job spec: output format, optional loudness
/// normalization, and embedded metadata. Carried by
/// [`AudioCommand::ExportAudio`](crate::types::AudioCommand::ExportAudio)
/// and (later) the stem-export command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSettings {
    pub format: ExportFormat,
    pub normalize: NormalizeSpec,
    pub metadata: ExportMetadata,
}

impl ExportSettings {
    /// The settings the legacy `BounceToWav` shim maps to: 32-bit-float
    /// WAV at the engine rate, no normalization, no metadata. Preserves
    /// the exact pre-export-pipeline bounce behaviour.
    pub fn default_wav() -> Self {
        ExportSettings {
            format: ExportFormat::default_wav(),
            normalize: NormalizeSpec::default(),
            metadata: ExportMetadata::default(),
        }
    }
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self::default_wav()
    }
}
