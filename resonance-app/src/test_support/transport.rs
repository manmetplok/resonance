//! Transport and global-timeline test hooks: tempo map / tempo and
//! signature events, playing/recording flags, playhead, loop state,
//! view mode and the Performance-mode footer.

use crate::state;
use crate::Resonance;

impl Resonance {
    #[doc(hidden)]
    pub fn test_tempo_map(&self) -> &resonance_audio::types::TempoMap {
        &self.tempo_map
    }

    #[doc(hidden)]
    pub fn test_tempo_events(&self) -> &[state::TempoEvent] {
        &self.tempo_events
    }

    #[doc(hidden)]
    pub fn test_signature_events(&self) -> &[state::SignatureEvent] {
        &self.signature_events
    }

    #[doc(hidden)]
    /// Test-only: the transport playhead in samples.
    #[doc(hidden)]
    pub fn test_transport_playhead(&self) -> u64 {
        self.transport.playhead
    }

    /// Test-only: the loop range `(loop_in, loop_out, enabled)`.
    #[doc(hidden)]
    pub fn test_loop_range(&self) -> (u64, u64, bool) {
        (
            self.transport.loop_in,
            self.transport.loop_out,
            self.transport.loop_enabled,
        )
    }

    pub fn test_transport_bpm(&self) -> f32 {
        self.transport.bpm
    }

    /// Test-only: cycle-record mode (W3) — see
    /// `state::TransportState::loop_record_mode`.
    #[doc(hidden)]
    pub fn test_loop_record_mode(&self) -> bool {
        self.transport.loop_record_mode
    }

    /// Test-only: the last `AudioEvent::IoLatencyReport` mirror (W3) —
    /// see `state::DeviceState::io_latency`.
    #[doc(hidden)]
    pub fn test_io_latency(&self) -> Option<state::IoLatencyInfo> {
        self.devices.io_latency
    }

    #[doc(hidden)]
    pub fn test_transport_time_sig(&self) -> (u8, u8) {
        (self.transport.time_sig_num, self.transport.time_sig_den)
    }

    #[doc(hidden)]
    pub fn test_selected_global_event(&self) -> Option<state::SelectedGlobalEvent> {
        self.ui.interaction.selected_global_event
    }

    /// Test-only: rebuild the GUI-side tempo map from the current
    /// `tempo_events` / `signature_events`. Mirrors what the global-
    /// track reducers call after a tempo edit; surfaced so tests can
    /// seed a custom tempo map without going through the message path.
    #[doc(hidden)]
    pub fn test_rebuild_tempo_map(&mut self) {
        self.rebuild_tempo_map();
    }

    /// Test-only: push a tempo event so the rebuilt tempo map has the
    /// requested ramp/step. Caller must follow with
    /// `test_rebuild_tempo_map` (and usually `test_set_sample_rate`).
    #[doc(hidden)]
    pub fn test_push_tempo_event(&mut self, event: state::TempoEvent) {
        self.tempo_events.push(event);
    }

    /// Test-only: replace the signature track. Caller must follow with
    /// `test_rebuild_tempo_map`.
    #[doc(hidden)]
    pub fn test_set_signature_events(&mut self, events: Vec<state::SignatureEvent>) {
        self.signature_events = events;
    }

    /// Test-only: set a flat project tempo (single bar-0 event) and
    /// rebuild the tempo map / bar table off the current sample rate.
    /// Used by the compose bar↔sample tests (ba todo #1163) to establish
    /// a tempo whose samples-per-bar is non-integral (e.g. 108 BPM at
    /// 48 kHz = 106666.67 samples/bar).
    #[doc(hidden)]
    pub fn test_set_flat_tempo(&mut self, bpm: f32) {
        self.transport.bpm = bpm;
        self.tempo_events = vec![state::TempoEvent { bar: 0, bpm }];
        self.rebuild_tempo_map();
    }

    /// Test-only: the currently active top-level [`ViewMode`].
    #[doc(hidden)]
    pub fn test_view_mode(&self) -> state::ViewMode {
        self.ui.view_mode
    }

    /// Test-only: directly set the active view (bypassing the reducer)
    /// to establish a starting tab for Performance-mode toggle tests.
    #[doc(hidden)]
    pub fn test_set_view_mode(&mut self, mode: state::ViewMode) {
        self.ui.view_mode = mode;
    }

    /// Test-only: whether the transport reports as playing. Used to
    /// assert that entering/leaving Performance mode never starts or
    /// stops playback.
    #[doc(hidden)]
    pub fn test_transport_playing(&self) -> bool {
        self.transport.playing
    }

    /// Test-only: force the transport's playing flag so a test can prove
    /// a view switch preserves it (no engine round-trip involved).
    #[doc(hidden)]
    pub fn test_set_transport_playing(&mut self, playing: bool) {
        self.transport.playing = playing;
    }

    /// Test-only: arm/disarm the first track's record flag so a test can
    /// assert that record-arm never auto-opens Performance mode.
    #[doc(hidden)]
    pub fn test_arm_first_track(&mut self, armed: bool) {
        if let Some(track) = self.registry.tracks.first_mut() {
            track.record_armed = armed;
        }
    }

    /// Test-only: read the Performance-mode footer selection (instrument
    /// tuning + capo, epic #11 / todo #312).
    #[doc(hidden)]
    pub fn test_performance(&self) -> &crate::state::PerformanceState {
        &self.performance
    }

    /// Test-only: mutable access to the Performance-mode footer selection,
    /// so a persistence test can seed a tuning / capo before serializing.
    #[doc(hidden)]
    pub fn test_performance_mut(&mut self) -> &mut crate::state::PerformanceState {
        &mut self.performance
    }

    /// Test-only: replay just the Performance-mode footer block of a saved
    /// [`crate::project::ProjectFile`] into this app, exercising the same
    /// restore path a full project load runs (ba todo #312).
    #[doc(hidden)]
    pub fn test_restore_performance(&mut self, file: &crate::project::ProjectFile) {
        crate::update::project_io::replay::restore_performance(self, file);
    }

    /// Test-only: force the transport's recording flag so a test can render
    /// the Performance status bar in its recording state.
    #[doc(hidden)]
    pub fn test_set_transport_recording(&mut self, recording: bool) {
        self.transport.recording = recording;
    }

    /// Test-only: the current transport playhead sample. Marker
    /// navigation reducers move this in lockstep with the `SeekTo`
    /// command sent to the engine.
    #[doc(hidden)]
    pub fn test_playhead(&self) -> u64 {
        self.transport.playhead
    }

    /// Test-only: the transport loop range / enabled flags
    /// `(loop_in, loop_out, loop_enabled)`. `LoopToRegion` sets these in
    /// lockstep with the `SetLoopRange` command sent to the engine.
    #[doc(hidden)]
    pub fn test_loop_state(&self) -> (u64, u64, bool) {
        (
            self.transport.loop_in,
            self.transport.loop_out,
            self.transport.loop_enabled,
        )
    }

    /// Test-only: the CPU readout's polled load (code review UX-11).
    #[doc(hidden)]
    pub fn test_cpu_load(&self) -> Option<crate::state::CpuLoad> {
        self.transport.cpu_load
    }

    /// Test-only: publish a DSP load on the engine handle as the mix
    /// callback would (the next tick polls it).
    #[doc(hidden)]
    pub fn test_publish_dsp_load(&self, smoothed: f32, peak: f32) {
        self.engine.__set_dsp_load_for_test(smoothed, peak);
    }

    /// Test-only: whether the BPM field holds uncommitted text.
    #[doc(hidden)]
    pub fn test_bpm_editing(&self) -> bool {
        self.transport.bpm_editing
    }
}
