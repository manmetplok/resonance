//! Clip state and the transient drag/trim state that the timeline
//! interaction code carries while the user is moving or resizing a clip.

use resonance_audio::types::*;

#[derive(Debug, Clone)]
pub struct ClipDragState {
    pub clip_id: ClipId,
    pub grab_offset_x: f32,
    pub original_track_id: TrackId,
    pub current_x: f32,
    pub current_y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipEdge {
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub struct ClipTrimState {
    pub clip_id: ClipId,
    pub edge: ClipEdge,
    pub original_start_sample: SamplePos,
    pub original_trim_start: u64,
    pub original_trim_end: u64,
    pub original_total_frames: u64,
    pub anchor_x: f32,
}

/// Transient state for an in-progress fade-handle drag on an audio clip.
///
/// Mirrors [`ClipTrimState`]: the fade handles ride along the clip's top
/// edge (`handle x = ramp end`, design doc #153), so the drag converts the
/// live pointer x into a fade length in frames against the clip's start
/// (fade-in / [`ClipEdge::Left`]) or end (fade-out / [`ClipEdge::Right`]).
/// The clip's geometry is anchored at grab time so the conversion is stable
/// even though the live [`ClipState`] is mutated as the drag progresses.
#[derive(Debug, Clone)]
pub struct FadeDragState {
    pub clip_id: ClipId,
    /// Which fade is being dragged: `Left` = fade-in, `Right` = fade-out.
    pub edge: ClipEdge,
    /// Clip start in samples at grab time (the fade-in anchor edge).
    pub original_start_sample: SamplePos,
    /// Clip visible duration in samples at grab time. Bounds the fade
    /// length (a fade can't be longer than the clip is audible) and gives
    /// the fade-out anchor edge (`start + duration`).
    pub original_duration_samples: u64,
}

/// Transient state for an in-progress clip-gain bead drag.
///
/// Gain is a vertical gesture (`ns-resize`): dragging up increases the gain
/// in dB, down decreases it. `anchor_y` is the pointer y at grab and
/// `original_gain_db` the clip's gain at that moment, so each move computes
/// an absolute target from the total vertical delta rather than
/// accumulating per-frame (which would drift).
#[derive(Debug, Clone)]
pub struct GainDragState {
    pub clip_id: ClipId,
    pub anchor_y: f32,
    pub original_gain_db: f32,
}

/// GUI-side clip state.
#[derive(Debug, Clone)]
pub struct ClipState {
    pub id: ClipId,
    pub track_id: TrackId,
    pub start_sample: SamplePos,
    pub duration_samples: u64,
    pub name: String,
    /// Total raw audio frames (before trim). Used for trim bounds.
    pub total_frames: u64,
    pub trim_start_frames: u64,
    pub trim_end_frames: u64,
    /// Fade-in length in frames; `0` = no fade. Mirrored from the engine.
    pub fade_in_frames: u64,
    /// Curve shaping the fade-in ramp.
    pub fade_in_curve: FadeCurve,
    /// Fade-out length in frames; `0` = no fade. Mirrored from the engine.
    pub fade_out_frames: u64,
    /// Curve shaping the fade-out ramp.
    pub fade_out_curve: FadeCurve,
    /// Per-clip gain in decibels. `0.0` dB = unity (no change).
    pub gain_db: f32,
    /// Downsampled waveform peaks: (min, max) per chunk of frames.
    pub waveform_peaks: Vec<(f32, f32)>,
    /// GUI-side mirror of the clip's non-destructive vocal-tuning model
    /// (doc #160). `None` means the clip has never been pitch-analysed —
    /// the common case for non-vocal audio, with zero overhead. The engine
    /// owns the authoritative copy on the `AudioClip`; this mirror is
    /// filled from `AudioEvent::ClipPitchDetected` so the pitch editor can
    /// read the detected contour / notes (and, later, the per-note and
    /// global edits) without a read-back round-trip.
    pub vocal_tuning: Option<VocalTuning>,
    /// Link to the media-pool asset this clip was placed from, if any
    /// (doc #175). `None` for clips that didn't originate from an import
    /// (recorded takes, bounced/rendered audio, legacy projects). An
    /// asset's usage count is the number of clips whose `asset_ref`
    /// points at it; the link is persisted in the project file and
    /// rebuilt on load so imported audio survives save/reload.
    pub asset_ref: Option<crate::state::pool::AssetRef>,
}

/// GUI-side MIDI clip state.
#[derive(Debug, Clone)]
pub struct MidiClipState {
    pub id: ClipId,
    pub track_id: TrackId,
    pub start_sample: SamplePos,
    pub duration_ticks: u64,
    pub name: String,
    pub notes: Vec<MidiNote>,
    pub trim_start_ticks: u64,
    pub trim_end_ticks: u64,
}

#[derive(Debug, Clone)]
pub struct MidiClipDragState {
    pub clip_id: ClipId,
    pub grab_offset_x: f32,
    pub original_track_id: TrackId,
    pub current_x: f32,
    pub current_y: f32,
}

#[derive(Debug, Clone)]
pub struct MidiClipTrimState {
    pub clip_id: ClipId,
    pub edge: ClipEdge,
    pub original_start_sample: SamplePos,
    pub original_duration_ticks: u64,
    pub original_trim_start_ticks: u64,
    pub original_trim_end_ticks: u64,
    pub anchor_x: f32,
}
