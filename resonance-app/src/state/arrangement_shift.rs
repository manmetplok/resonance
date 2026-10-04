//! The report of the last `arrangement.insert_bars` / `remove_bars`
//! (`UiTransientState::last_arrangement_shift`). Defined here rather than
//! in `update::arrangement`, which produces it, because state owns the
//! types it holds (ARCH2-05).

use resonance_audio::types::ClipId;

/// One structural shift's outcome, as the control layer reports it.
#[derive(Debug, Clone, Default)]
pub struct ShiftOutcome {
    pub shift_samples: i64,
    pub audio_clips_moved: u32,
    pub midi_clips_moved: u32,
    pub placements_moved: u32,
    pub markers_moved: u32,
    pub automation_points_moved: u32,
    pub tempo_events_moved: u32,
    pub signature_events_moved: u32,
    /// Events superseded on the bar they were clamped onto — see the
    /// last-wins rule in `update::arrangement`'s module docs. Always 0 for
    /// `insert_bars`.
    pub tempo_events_removed: u32,
    pub signature_events_removed: u32,
    pub clips_deleted: Vec<ClipId>,
    pub placements_deleted: Vec<u64>,
}
