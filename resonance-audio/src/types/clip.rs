//! Audio and MIDI clip data structures, plus the waveform peak helper.
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

/// An audio clip stored in memory.
#[derive(Debug)]
pub struct AudioClip {
    pub id: ClipId,
    pub track_id: TrackId,
    /// Start position on the timeline in samples.
    pub start_sample: SamplePos,
    /// Decoded audio data: stereo interleaved f32 samples.
    pub data: Vec<f32>,
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
        (self.data.len() / 2) as u64
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
