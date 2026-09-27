//! WAV file → wavetable frames.
//!
//! Decoding is `hound`'s; this module decides how the decoded samples become
//! frames. The rules, in order:
//!
//! 1. A `clm ` chunk (Serum's marker, `<!>2048 …`) names the frame length —
//!    honoured when it is a sane size, and the frames are resampled to
//!    `WAVETABLE_SIZE` if it differs.
//! 2. Otherwise the file is read as `WAVETABLE_SIZE`-sample frames (the Serum
//!    convention every wavetable editor exports).
//! 3. A file too short to be frames — under one frame, or under two whole
//!    frames and not a multiple of the frame length — is a *single cycle* of
//!    whatever length it has, resampled to `WAVETABLE_SIZE`.
//! 4. A longer file that is not a whole number of frames drops its tail.
//! 5. More than [`MAX_USER_FRAMES`] frames are thinned evenly across the file.
//!
//! Every frame then loses its DC offset, and the table as a whole is
//! normalised to unit peak (one gain for every frame, so a deliberately quiet
//! frame stays quiet relative to its neighbours until the mip builder
//! normalises each level, as it does for the bundled tables).
//!
//! Multi-channel files are mixed to mono.

use std::io::Cursor;
use std::path::Path;

use crate::dsp::user_table::{resample_cycle, MAX_USER_FRAMES};
use crate::dsp::wavetable::WAVETABLE_SIZE;

/// Frame lengths a `clm ` chunk may name. Anything outside this is a chunk
/// from a tool that means something else by it, and is ignored.
const CLM_FRAME_RANGE: std::ops::RangeInclusive<usize> = 16..=65_536;

/// Peak below which an import is treated as silence and refused.
const SILENCE: f32 = 1e-6;

/// The frames an import produced, ready for [`crate::dsp::user_table::UserTable::build`].
#[derive(Debug, Clone)]
pub struct ImportedFrames {
    /// `num_frames × WAVETABLE_SIZE` samples, DC-free, peak-normalised.
    pub frames: Vec<f32>,
    pub num_frames: usize,
    /// How many frames the file held before thinning (equal to `num_frames`
    /// unless the file had more than [`MAX_USER_FRAMES`]).
    pub source_frames: usize,
    /// The frame length the file was sliced at — the `clm ` size, the
    /// convention's `WAVETABLE_SIZE`, or the whole file for a single cycle.
    pub source_frame_size: usize,
}

/// Read and import a WAV file.
pub fn import_wav_file(path: &Path) -> Result<ImportedFrames, String> {
    let bytes = std::fs::read(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("file not found: {}", path.display())
        } else {
            format!("could not read {}: {e}", path.display())
        }
    })?;
    import_wav_bytes(&bytes)
}

/// Import a WAV file already in memory.
pub fn import_wav_bytes(bytes: &[u8]) -> Result<ImportedFrames, String> {
    let samples = decode_mono(bytes)?;
    slice_frames(&samples, clm_frame_size(bytes))
}

/// Slice mono `samples` into wavetable frames by the rules in the module
/// docs. `frame_size` is the file's declared frame length, if it has one.
pub fn slice_frames(samples: &[f32], frame_size: Option<usize>) -> Result<ImportedFrames, String> {
    if samples.len() < 2 {
        return Err("the file holds no audio".to_string());
    }
    let declared = frame_size.filter(|n| CLM_FRAME_RANGE.contains(n));
    let size = declared.unwrap_or(WAVETABLE_SIZE);
    let whole = samples.len() / size;
    let single_cycle = match declared {
        Some(_) => whole == 0,
        None => samples.len() % size != 0 && samples.len() < 2 * size,
    };

    let (sources, source_frame_size): (Vec<&[f32]>, usize) = if single_cycle {
        (vec![samples], samples.len())
    } else {
        (samples.chunks_exact(size).collect(), size)
    };
    let source_frames = sources.len();
    let picked: Vec<&[f32]> = if source_frames > MAX_USER_FRAMES {
        (0..MAX_USER_FRAMES)
            .map(|i| {
                let span = source_frames - 1;
                let at = (i * span + (MAX_USER_FRAMES - 1) / 2) / (MAX_USER_FRAMES - 1);
                sources[at]
            })
            .collect()
    } else {
        sources
    };

    let mut frames = Vec::with_capacity(picked.len() * WAVETABLE_SIZE);
    for src in &picked {
        let mut frame = if src.len() == WAVETABLE_SIZE {
            src.to_vec()
        } else {
            resample_cycle(src)
        };
        let mean = frame.iter().map(|&s| s as f64).sum::<f64>() / frame.len() as f64;
        for s in frame.iter_mut() {
            *s -= mean as f32;
        }
        frames.extend_from_slice(&frame);
    }

    let peak = frames.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if !peak.is_finite() {
        return Err("the file holds non-finite samples".to_string());
    }
    if peak < SILENCE {
        return Err("the file is silent".to_string());
    }
    let inv = 1.0 / peak;
    for s in frames.iter_mut() {
        *s *= inv;
    }

    Ok(ImportedFrames {
        num_frames: picked.len(),
        frames,
        source_frames,
        source_frame_size,
    })
}

/// Decode a WAV file to mono `f32`, averaging the channels.
fn decode_mono(bytes: &[u8]) -> Result<Vec<f32>, String> {
    let reader = hound::WavReader::new(Cursor::new(bytes))
        .map_err(|e| format!("not a readable WAV file: {e}"))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| format!("corrupt WAV data: {e}"))?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()
                .map_err(|e| format!("corrupt WAV data: {e}"))?
        }
    };
    let inv = 1.0 / channels as f32;
    Ok(interleaved
        .chunks_exact(channels)
        .map(|c| c.iter().sum::<f32>() * inv)
        .collect())
}

/// The frame length a Serum-style `clm ` chunk declares (`<!>2048 …`), if
/// the file has one.
pub fn clm_frame_size(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut off = 12;
    while off + 8 <= bytes.len() {
        let id = &bytes[off..off + 4];
        let len = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().ok()?) as usize;
        let body = off + 8;
        let end = body.checked_add(len)?.min(bytes.len());
        if id == b"clm " {
            let text = std::str::from_utf8(&bytes[body..end]).ok()?;
            let digits: String = text
                .strip_prefix("<!>")?
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            return digits.parse().ok();
        }
        // Chunks are padded to an even length.
        off = body.checked_add(len + (len & 1))?;
    }
    None
}
