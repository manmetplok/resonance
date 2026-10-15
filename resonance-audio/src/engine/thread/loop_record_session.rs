//! The engine thread's bookkeeping for a cycle-record run.

use resonance_common::TimelineRange;

/// Bookkeeping for an in-flight cycle-record (loop-record) run. Created
/// in `transport::open_recording_session` when loop-record mode and a
/// loop range are both active, advanced at each loop seam, and torn down
/// when recording stops. Lives on the engine control thread so it never
/// touches the audio callback.
pub(crate) struct LoopRecordSession {
    /// The loop region being cycled over, in sample frames. Reported on
    /// every `AudioEvent::TakeCaptured` as the take's slot.
    pub slot: TimelineRange,
    /// Zero-based index of the MIDI pass currently being captured, bumped
    /// at each MIDI pass roll. Audio passes are cut by sample count and
    /// counted by `RecordingState::audio_passes_rolled` (code review
    /// RT-02).
    pub pass_index: u32,
    /// `SharedState::loop_wrap_count` when the session opened: a note
    /// stamped with it was played in the punch-in pass (BUG-22).
    pub first_wrap: u64,
    /// The loop-wrap count the open MIDI pass belongs to. A live note
    /// stamped with a later count, or the engine loop seeing the shared
    /// count move past it, rolls the pass (BUG-22).
    pub midi_wrap: u64,
}
