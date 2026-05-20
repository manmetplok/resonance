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
    pub recording_start_sample: u64,
    pub playhead: u64,
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
}

impl Default for TransportState {
    fn default() -> Self {
        Self {
            playing: false,
            recording: false,
            recording_start_sample: 0,
            playhead: 0,
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
        }
    }
}
