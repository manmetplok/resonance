//! Audio and MIDI clip data structures, plus the waveform peak helper.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{ClipId, SamplePos, TrackId};

/// A single MIDI note in a clip.
#[derive(Debug, Clone)]
pub struct MidiNote {
    pub note: u8,
    pub velocity: f32,
    pub start_tick: u64,
    pub duration_ticks: u64,
}

/// A MIDI clip containing note data, placed on the timeline.
#[derive(Debug)]
pub struct MidiClip {
    pub id: ClipId,
    pub track_id: TrackId,
    /// Position on the timeline in samples (same units as AudioClip).
    pub start_sample: SamplePos,
    /// Logical length in ticks.
    pub duration_ticks: u64,
    /// Notes sorted by start_tick.
    pub notes: Vec<MidiNote>,
    pub name: String,
    pub trim_start_ticks: u64,
    pub trim_end_ticks: u64,
}

impl MidiClip {
    /// Visible duration in ticks after trim.
    pub fn visible_duration_ticks(&self) -> u64 {
        self.duration_ticks
            .saturating_sub(self.trim_start_ticks)
            .saturating_sub(self.trim_end_ticks)
    }

    /// Convert visible duration to samples using the tempo map.
    pub fn duration_samples(&self, samples_per_tick: f64) -> u64 {
        (self.visible_duration_ticks() as f64 * samples_per_tick) as u64
    }

    /// End position on timeline in samples.
    pub fn end_sample(&self, samples_per_tick: f64) -> SamplePos {
        self.start_sample + self.duration_samples(samples_per_tick)
    }
}

/// A note event to be sent to a plugin during audio processing.
#[derive(Debug, Clone)]
pub struct PendingNoteEvent {
    pub is_note_on: bool,
    pub note: u8,
    pub velocity: f32,
    pub sample_offset: u32,
}

/// Backing storage for a clip's PCM samples. Recorded clips and
/// clips loaded from a project on disk are `Mapped` (memory-mapped
/// WAV files); clips that were just decoded in memory but not yet
/// persisted can also use `Memory` as a transient fallback.
#[derive(Debug)]
pub enum ClipSource {
    /// Owned, in-RAM stereo-interleaved f32 samples.
    Memory(Vec<f32>),
    /// Memory-mapped WAV file sliced down to the PCM data chunk.
    /// `data_offset_bytes` is the byte offset from the start of the
    /// mapping where interleaved f32 samples begin; `frame_count` is
    /// the number of stereo frames stored in the data chunk. `path`
    /// is the on-disk location of the file backing the mapping,
    /// retained so save-to-a-new-directory can copy it.
    Mapped {
        mmap: Arc<memmap2::Mmap>,
        data_offset_bytes: usize,
        frame_count: u64,
        path: PathBuf,
    },
}

impl ClipSource {
    /// Stereo-interleaved f32 samples as a slice: one `[l, r]` pair
    /// per frame. This is called from the mixer hot path, so it must
    /// be O(1) and allocation-free.
    #[inline]
    pub fn as_frames(&self) -> &[f32] {
        match self {
            ClipSource::Memory(v) => v.as_slice(),
            ClipSource::Mapped {
                mmap,
                data_offset_bytes,
                frame_count,
                ..
            } => {
                let byte_len = (*frame_count as usize) * 2 * std::mem::size_of::<f32>();
                let bytes = &mmap[*data_offset_bytes..*data_offset_bytes + byte_len];
                bytemuck::cast_slice::<u8, f32>(bytes)
            }
        }
    }

    /// Total number of stereo frames in the underlying PCM data.
    #[inline]
    pub fn frame_count(&self) -> u64 {
        match self {
            ClipSource::Memory(v) => (v.len() / 2) as u64,
            ClipSource::Mapped { frame_count, .. } => *frame_count,
        }
    }

    /// Open a 32-bit-float stereo WAV file, memory-map it, and
    /// return a `Mapped` ClipSource referencing its PCM data chunk.
    /// Also pre-touches every page to avoid major page faults on
    /// the audio thread the first time the clip is played.
    pub fn open_wav(path: &Path) -> Result<Self, String> {
        let file =
            std::fs::File::open(path).map_err(|e| format!("open wav {}: {e}", path.display()))?;
        let mmap = unsafe { memmap2::Mmap::map(&file) }
            .map_err(|e| format!("mmap {}: {e}", path.display()))?;

        let (data_offset_bytes, data_len_bytes) = locate_wav_float_data(&mmap)
            .map_err(|e| format!("parse wav {}: {e}", path.display()))?;
        if data_len_bytes % (2 * std::mem::size_of::<f32>()) != 0 {
            return Err(format!(
                "parse wav {}: data chunk length {} not a multiple of stereo f32 frames",
                path.display(),
                data_len_bytes
            ));
        }
        let frame_count = (data_len_bytes / (2 * std::mem::size_of::<f32>())) as u64;

        // Pre-touch: read one byte per 4 KiB page across the data
        // chunk so that the first mixer access doesn't trigger
        // major page faults on the realtime audio thread.
        pre_touch(&mmap[data_offset_bytes..data_offset_bytes + data_len_bytes]);

        Ok(ClipSource::Mapped {
            mmap: Arc::new(mmap),
            data_offset_bytes,
            frame_count,
            path: path.to_path_buf(),
        })
    }

    /// On-disk path backing a `Mapped` source, or `None` for `Memory`.
    pub fn mapped_path(&self) -> Option<&Path> {
        match self {
            ClipSource::Mapped { path, .. } => Some(path.as_path()),
            ClipSource::Memory(_) => None,
        }
    }
}

/// Parse a minimal RIFF/WAVE header and return the byte offset and
/// length of the PCM `data` chunk, verifying that the format chunk
/// declares 32-bit IEEE float stereo. Does not depend on `hound`.
fn locate_wav_float_data(bytes: &[u8]) -> Result<(usize, usize), String> {
    if bytes.len() < 12 {
        return Err("file too short".into());
    }
    if &bytes[0..4] != b"RIFF" {
        return Err("missing RIFF header".into());
    }
    if &bytes[8..12] != b"WAVE" {
        return Err("not a WAVE file".into());
    }

    let mut cursor = 12usize;
    let mut fmt_ok = false;
    while cursor + 8 <= bytes.len() {
        let id = &bytes[cursor..cursor + 4];
        let size = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        let chunk_start = cursor + 8;
        let chunk_end = chunk_start + size;
        if chunk_end > bytes.len() {
            return Err(format!("chunk {:?} overruns file", std::str::from_utf8(id)));
        }

        if id == b"fmt " {
            if size < 16 {
                return Err("fmt chunk too small".into());
            }
            let format =
                u16::from_le_bytes(bytes[chunk_start..chunk_start + 2].try_into().unwrap());
            let channels =
                u16::from_le_bytes(bytes[chunk_start + 2..chunk_start + 4].try_into().unwrap());
            let bits_per_sample = u16::from_le_bytes(
                bytes[chunk_start + 14..chunk_start + 16]
                    .try_into()
                    .unwrap(),
            );
            // `hound` writes float WAVs using WAVE_FORMAT_EXTENSIBLE
            // (0xFFFE) with a SubFormat GUID. The first two bytes
            // of that GUID carry the real format code, so we
            // inspect them instead of the outer format tag.
            const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
            const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
            let effective_format = if format == WAVE_FORMAT_EXTENSIBLE {
                if size < 40 {
                    return Err("extensible fmt chunk too small".into());
                }
                u16::from_le_bytes(
                    bytes[chunk_start + 24..chunk_start + 26]
                        .try_into()
                        .unwrap(),
                )
            } else {
                format
            };
            if effective_format != WAVE_FORMAT_IEEE_FLOAT {
                return Err(format!(
                    "unsupported format code {} (expected 3, IEEE float)",
                    effective_format
                ));
            }
            if channels != 2 {
                return Err(format!("expected stereo, got {} channels", channels));
            }
            if bits_per_sample != 32 {
                return Err(format!(
                    "expected 32-bit float, got {} bits",
                    bits_per_sample
                ));
            }
            fmt_ok = true;
        } else if id == b"data" {
            if !fmt_ok {
                return Err("data chunk before fmt chunk".into());
            }
            return Ok((chunk_start, size));
        }

        // RIFF chunks are word-aligned: an odd size is padded.
        cursor = chunk_end + (size & 1);
    }
    Err("no data chunk found".into())
}

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

/// An audio clip on the timeline. The PCM samples live behind
/// [`ClipSource`], which may be an owned `Vec<f32>` or a
/// memory-mapped WAV file — so large recorded takes never need to
/// inflate into a contiguous in-RAM buffer.
#[derive(Debug)]
pub struct AudioClip {
    pub id: ClipId,
    pub track_id: TrackId,
    /// Start position on the timeline in samples.
    pub start_sample: SamplePos,
    pub source: ClipSource,
    /// Original file name.
    pub name: String,
    /// Non-destructive trim: frames to skip from the start of audio data.
    pub trim_start_frames: u64,
    /// Non-destructive trim: frames to skip from the end of audio data.
    pub trim_end_frames: u64,
}

/// Number of stereo frames per waveform peak bucket.
pub const WAVEFORM_PEAK_FRAMES: usize = 512;

/// Compute downsampled waveform peaks from stereo interleaved audio data.
/// Returns (min, max) pairs, one per chunk of `WAVEFORM_PEAK_FRAMES` frames.
/// Uses the mono mix (L+R)/2 for display.
pub fn compute_waveform_peaks(data: &[f32]) -> Vec<(f32, f32)> {
    let total_frames = data.len() / 2;
    let num_peaks = (total_frames + WAVEFORM_PEAK_FRAMES - 1) / WAVEFORM_PEAK_FRAMES;
    let mut peaks = Vec::with_capacity(num_peaks);
    for chunk_start in (0..total_frames).step_by(WAVEFORM_PEAK_FRAMES) {
        let chunk_end = (chunk_start + WAVEFORM_PEAK_FRAMES).min(total_frames);
        let mut min_val = f32::MAX;
        let mut max_val = f32::MIN;
        for f in chunk_start..chunk_end {
            let mono = (data[f * 2] + data[f * 2 + 1]) * 0.5;
            if mono < min_val {
                min_val = mono;
            }
            if mono > max_val {
                max_val = mono;
            }
        }
        peaks.push((min_val, max_val));
    }
    peaks
}

impl AudioClip {
    /// Total number of frames in the raw audio data.
    pub fn total_frames(&self) -> u64 {
        self.source.frame_count()
    }

    /// Visible/audible duration in stereo sample frames (after trim).
    pub fn duration_frames(&self) -> u64 {
        self.total_frames()
            .saturating_sub(self.trim_start_frames)
            .saturating_sub(self.trim_end_frames)
    }

    /// End position on timeline in sample frames.
    pub fn end_sample(&self) -> SamplePos {
        self.start_sample + self.duration_frames()
    }
}
