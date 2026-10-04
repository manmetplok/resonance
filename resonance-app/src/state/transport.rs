//! Transport, tempo, metronome, and loop-range state.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoopDragTarget {
    In,
    Out,
}

/// Transport, tempo, metronome, and loop range — everything the play head
/// and the tempo engine depend on. Held as a sub-struct on `Resonance` so
/// handlers that only care about transport can take `&mut TransportState`.
#[derive(Debug, Clone)]
pub struct TransportState {
    pub playing: bool,
    pub recording: bool,
    /// A Record was asked for and the engine has not reported the pass
    /// started yet (the count-in).
    pub record_pending: bool,
    pub recording_start_sample: u64,
    pub playhead: u64,
    /// Where the current playback started (set by Play, Record and the
    /// Play / Stop toggle); the toggle's stop returns here.
    pub play_start: u64,
    pub bpm: f32,
    pub bpm_input: String,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
    pub metronome_enabled: bool,
    /// Number of bars the metronome counts in before playback/recording
    /// starts. 0 disables the pre-count.
    pub precount_bars: u8,
    pub loop_enabled: bool,
    pub loop_in: u64,
    pub loop_out: u64,
    pub loop_range_set: bool,
    pub dragging_loop: Option<LoopDragTarget>,
    /// Cycle-record mode mirror (`AudioCommand::SetLoopRecordMode`):
    /// true rolls each loop pass into its own take at the seam (comp/solo
    /// later), false keeps the legacy single merged clip for the whole
    /// cycle-recorded run. Persisted on the project (W3) — it decides what
    /// shape the recorded content takes, same as the loop range itself,
    /// not a cross-project device preference.
    pub loop_record_mode: bool,
    /// The BPM field holds typed text not yet committed with Enter (code
    /// review UX-15). Set by `SetBpmText`; cleared by the commit and by
    /// [`Self::revert_bpm_text`]. While set, a press off the field reverts
    /// it, so the field never shows a tempo the song is not playing at.
    pub bpm_editing: bool,
    /// The pointer is over the BPM field (see [`Self::bpm_editing`]).
    pub bpm_hovered: bool,
    /// The mixer's DSP load, polled from the engine every tick for the
    /// transport's CPU readout (code review UX-11). `None` until the
    /// engine has published a measurement (no audio callback has run).
    pub cpu_load: Option<CpuLoad>,
}

/// One reading of the realtime mix callback's load, as fractions of the
/// cycle budget (1.0 = the whole budget — an xrun).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CpuLoad {
    /// Smoothed load (≈⅓ s EMA).
    pub smoothed: f32,
    /// Highest single cycle in the engine's current report window.
    pub peak: f32,
}

impl TransportState {
    /// Recording, or counting in to record.
    pub fn is_recording(&self) -> bool {
        self.recording || self.record_pending
    }

    /// Drop the BPM field's uncommitted text: show the song tempo again
    /// (code review UX-15).
    pub fn revert_bpm_text(&mut self) {
        if self.bpm_editing {
            self.bpm_input = format!("{:.1}", self.bpm);
            self.bpm_editing = false;
        }
    }
}

impl Default for TransportState {
    fn default() -> Self {
        Self {
            playing: false,
            recording: false,
            record_pending: false,
            recording_start_sample: 0,
            playhead: 0,
            play_start: 0,
            bpm: 120.0,
            bpm_input: "120".to_string(),
            time_sig_num: 4,
            time_sig_den: 4,
            metronome_enabled: false,
            precount_bars: 2,
            loop_enabled: false,
            loop_in: 0,
            loop_out: 0,
            loop_range_set: false,
            dragging_loop: None,
            loop_record_mode: false,
            bpm_editing: false,
            bpm_hovered: false,
            cpu_load: None,
        }
    }
}
