//! Engine → GUI event enum.
use resonance_common::{
    AudioFormat, AutomationLane, AutomationTarget, BindingId, ControlSource, ExternalInstrument,
    MidiBinding, MidiTarget, TakeContent, TakeGroupId, TimelineRange,
};
use resonance_metering::MeterSnapshot;

use crate::midi_hardware::MidiDeviceInfo;

use super::{
    ABSource, AssetId, BusId, ClipId, F0Frame, FadeCurve, InputDeviceInfo, MidiNote, NoteBlob,
    ParamInfo, PluginInstanceId, ReferenceAnalysisStage, ReferenceId, SamplePos, ScannedPlugin,
    SendId, SendSource, TrackId, WarpAlgorithm, WarpMarker,
};
use crate::quantize::GrooveTemplate;
use resonance_common::FreezeCacheRef;

/// Lifecycle stage of a single file in an `ImportAudioToPool` batch.
/// Drives the import/transcode progress modal: every file is reported
/// `Queued` up front, flips to `Working` while its worker decodes and
/// transcodes, and lands on `Done` once its `AssetImported` event has
/// been emitted. A file that fails reports no `Done` — it terminates
/// with [`AudioEvent::ImportFailed`] instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStage {
    Queued,
    Working,
    Done,
}

/// Inline clip payload for the offline "bounce in place" flow. The
/// realtime flow leaves this `None` because the clip arrives via the
/// regular `RecordingFinished` channel.
#[derive(Debug, Clone)]
pub struct BouncedClipData {
    pub clip_id: ClipId,
    pub start_sample: SamplePos,
    pub duration_samples: u64,
    pub name: String,
    /// Downsampled waveform peaks: (min, max) per chunk of frames.
    pub waveform_peaks: Vec<(f32, f32)>,
}

/// Which pass of an [`AudioEvent::ExportProgress`] update is running.
/// Non-normalized and true-peak exports run a single `Render`/`Encode`
/// sweep; integrated-LUFS normalization adds an `Analyze` pass up front
/// (see doc #196).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportPhase {
    /// Rendering the project mix through the chunked render core.
    Render,
    /// Measuring integrated loudness / true peak before the gain trim.
    Analyze,
    /// Feeding rendered frames to the encoder sink.
    Encode,
}

/// Why an [`AudioEvent::ExportError`] fired. Lets the app distinguish a
/// recoverable encoder-unavailable case (offer the WAV/FLAC fallback)
/// from hard I/O failures, an empty project, a running transport, or a
/// user cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportErrorKind {
    /// The selected format's encoder is unavailable (e.g. an optional
    /// native library was not compiled in). Surfaced *before* any partial
    /// file is written so the app can offer the always-available fallback.
    EncoderUnavailable,
    /// Filesystem error creating/writing/finalizing the output file.
    Io,
    /// The export was cancelled via `AudioCommand::CancelBounce`.
    Cancelled,
    /// The project has no audio to render.
    NoAudio,
    /// The transport is rolling; stop it before exporting.
    TransportRunning,
}

/// Events sent from the audio engine back to the GUI.
#[derive(Debug, Clone)]
pub enum AudioEvent {
    PlayheadMoved(SamplePos),
    SampleRateDetected {
        sample_rate: u32,
    },
    ClipImported {
        clip_id: ClipId,
        track_id: TrackId,
        start_sample: SamplePos,
        duration_samples: u64,
        name: String,
        /// Downsampled waveform peaks: (min, max) per chunk of frames.
        waveform_peaks: Vec<(f32, f32)>,
    },
    /// Per-file progress for an `ImportAudioToPool` batch. See
    /// [`ImportStage`]. `asset_id` matches the eventual
    /// [`AudioEvent::AssetImported`] / [`AudioEvent::ImportFailed`] for
    /// the same file, so the progress modal can key its rows by id.
    ImportProgress {
        asset_id: AssetId,
        /// Original source path, as passed in the command.
        path: String,
        stage: ImportStage,
    },
    /// A source file was successfully imported into the project pool.
    /// The engine-format WAV now lives at `project_relative_path` (e.g.
    /// `"audio/asset_7.wav"`) inside the project directory, stereo f32 at
    /// the project sample rate. `channels` and `source_sample_rate`
    /// describe the *original* file (for display, e.g. "Mono · 44.1 kHz");
    /// `duration_frames` is the per-channel frame count of the imported
    /// (project-rate) WAV, i.e. what a placed clip would span.
    AssetImported {
        asset_id: AssetId,
        project_relative_path: String,
        original_path: String,
        format: AudioFormat,
        channels: u16,
        source_sample_rate: u32,
        duration_frames: u64,
        /// Downsampled waveform peaks: (min, max) per chunk of frames.
        peaks: Vec<(f32, f32)>,
    },
    /// A source file in an `ImportAudioToPool` batch failed to import
    /// (decode/transcode error, missing file, …). `reason` is
    /// user-facing. The batch continues with the remaining files.
    ImportFailed {
        asset_id: AssetId,
        path: String,
        reason: String,
    },
    TrackAdded {
        track_id: TrackId,
    },
    TrackRemoved {
        track_id: TrackId,
    },
    ClipDeleted {
        clip_id: ClipId,
    },
    ClipMoved {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        new_track_id: TrackId,
    },
    ClipTrimmed {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        new_duration_samples: u64,
        trim_start_frames: u64,
        trim_end_frames: u64,
    },
    /// A clip's fade-in/out lengths and/or curves changed. Carries the
    /// engine-clamped values so the app mirror matches engine state.
    ClipFadeChanged {
        clip_id: ClipId,
        fade_in_frames: u64,
        fade_in_curve: FadeCurve,
        fade_out_frames: u64,
        fade_out_curve: FadeCurve,
    },
    /// A clip's per-clip gain changed. Carries the engine-clamped dB value.
    ClipGainChanged {
        clip_id: ClipId,
        gain_db: f32,
    },
    /// A clip's warp ("follow tempo") parameters changed. Carries the
    /// engine-stored values so the app mirror matches engine state.
    ClipWarpChanged {
        clip_id: ClipId,
        warp_enabled: bool,
        original_bpm: Option<f32>,
        transpose_semitones: f32,
        warp_algorithm: WarpAlgorithm,
    },
    /// A clip's warp-marker set changed (full-set replace). Carries the
    /// engine-sorted markers (ascending `timeline_beat`) so the app
    /// mirror matches engine state.
    ClipWarpMarkersChanged {
        clip_id: ClipId,
        markers: Vec<WarpMarker>,
    },

    /// Tempo/BPM detection finished for an audio clip. The detector
    /// ran over the clip's source samples and estimated a tempo and
    /// confidence. The app may use this to populate the clip's
    /// `original_bpm` via [`AudioCommand::SetClipWarp`]; the engine
    /// itself does not mutate the clip.
    ClipTempoDetected {
        clip_id: ClipId,
        bpm: f32,
        confidence: f32,
    },
    /// Vocal pitch analysis finished for a clip (`AnalyzeClipPitch`). The
    /// detected f0 `contour` and segmented `notes` mirror what the engine
    /// stored in the clip's [`VocalTuning`](super::VocalTuning) cache, so
    /// the app can update its own mirror without a read-getter. Both
    /// vectors may be empty when the clip carries no voiced material.
    ClipPitchDetected {
        clip_id: ClipId,
        notes: Vec<NoteBlob>,
        contour: Vec<F0Frame>,
    },
    /// An automation lane was stored or replaced (or its read flag
    /// toggled). Carries the lane exactly as the engine holds it — points
    /// sorted, `enabled` reflecting the current read state — so the app
    /// mirror matches engine state.
    AutomationLaneChanged {
        lane: AutomationLane,
    },
    /// The automation lane for `target` was removed from engine state.
    AutomationLaneCleared {
        target: AutomationTarget,
    },
    Stopped,
    Error(String),
    InputDevicesListed {
        devices: Vec<InputDeviceInfo>,
        default_name: Option<String>,
    },
    RecordingStarted {
        start_sample: SamplePos,
    },
    RecordingFinished {
        clip_id: ClipId,
        track_id: TrackId,
        start_sample: SamplePos,
        duration_samples: u64,
        name: String,
        /// Downsampled waveform peaks: (min, max) per chunk of frames.
        waveform_peaks: Vec<(f32, f32)>,
    },
    /// One loop pass of a cycle-record run was captured into a distinct
    /// take. Emitted per armed track at each loop seam (and once more for
    /// the trailing pass when recording stops) while
    /// `AudioCommand::SetLoopRecordMode(true)` is active. `group_id` is
    /// stable for a track across all passes of one record run, so the app
    /// can fold passes 0..N of a track into a single take group.
    /// `content` carries the audio clip reference or the captured MIDI
    /// notes depending on the track type.
    TakeCaptured {
        group_id: TakeGroupId,
        track_id: TrackId,
        /// The loop region the take was recorded over, in sample frames.
        slot: TimelineRange,
        /// Zero-based loop pass that produced this take.
        pass_index: u32,
        content: TakeContent,
    },
    PluginAdded {
        track_id: TrackId,
        instance_id: PluginInstanceId,
        plugin_name: String,
        clap_plugin_id: String,
        clap_file_path: String,
        params: Vec<ParamInfo>,
        /// Whether the plugin exposes a CLAP GUI the host can open.
        has_gui: bool,
        /// Number of audio output ports declared by this plugin instance.
        /// `1` for every legacy single-output plugin. Multi-output plugins
        /// (e.g. `resonance-drums`) report a larger number and the app
        /// auto-creates one sub-track per non-main output port.
        output_port_count: usize,
        /// Human-readable name of each output port, same length as
        /// `output_port_count`.
        output_port_names: Vec<String>,
    },
    PluginRemoved {
        track_id: TrackId,
        instance_id: PluginInstanceId,
    },
    PluginsScanned {
        plugins: Vec<ScannedPlugin>,
    },
    PluginStateSaved {
        instance_id: PluginInstanceId,
        data: Vec<u8>,
    },
    BounceComplete {
        path: String,
    },
    BounceError(String),

    // -- Stem export (multi-target offline render) --
    /// The stem export could not start at all (transport rolling, empty
    /// range, or no targets). No files were written. The string is
    /// user-facing. Per-target failures use `StemExportTargetError`.
    StemExportError(String),
    /// A stem export target is starting. `target_index` is its 0-based
    /// position in the queue, `total` the number of targets, and
    /// `fraction` the overall queue progress in `[0.0, 1.0]` at the
    /// moment this target begins.
    StemExportProgress {
        target_index: usize,
        total: usize,
        fraction: f32,
    },
    /// One stem target finished rendering and its WAV is on disk.
    StemExportTargetDone {
        index: usize,
        path: String,
    },
    /// One stem target failed to render or write. The export KEEPS every
    /// stem written so far and continues with the remaining targets, so
    /// the app can offer "retry remaining". The string is user-facing.
    StemExportTargetError {
        index: usize,
        message: String,
    },
    /// The whole stem export finished. `files` lists every WAV actually
    /// written, in queue order (a target that errored is absent).
    StemExportComplete {
        files: Vec<String>,
    },
    /// The stem export was cancelled (via `AudioCommand::CancelStemExport`)
    /// between targets. Stems already written stay on disk and are listed
    /// in `files`; the in-flight target, if any, is not.
    StemExportCancelled {
        files: Vec<String>,
    },
    /// Progress update for an [`AudioCommand::ExportAudio`] job.
    /// `fraction` is in `[0.0, 1.0]` within the reported `phase`.
    /// Generalizes `BounceProgress` for the export pipeline; the legacy
    /// WAV bounce shim keeps emitting `BounceProgress` unchanged.
    ExportProgress {
        phase: ExportPhase,
        fraction: f32,
    },
    /// An [`AudioCommand::ExportAudio`] job finished successfully. The
    /// loudness figures are populated when normalization ran (`None`/`0.0`
    /// otherwise) so the app can show the verified achieved loudness.
    ExportComplete {
        path: String,
        /// Achieved integrated loudness in LUFS, when measured.
        achieved_lufs: Option<f32>,
        /// Achieved true peak in dBTP.
        achieved_dbtp: f32,
        /// Encoded file size in bytes.
        bytes: u64,
    },
    /// An [`AudioCommand::ExportAudio`] job failed (or was cancelled).
    /// `message` is user-facing; `kind` lets the app react structurally.
    ExportError {
        kind: ExportErrorKind,
        message: String,
    },
    /// "Bounce in place" finished. Covers both the offline (internal
    /// synth) and realtime (external MIDI) flows; `clip` is `Some` when
    /// the engine rendered the clip inline (offline) and `None` when
    /// the realtime branch delivered it via the existing
    /// `RecordingFinished` event. The app mirrors the source mute
    /// locally and reorders the new track to sit beneath the source.
    TrackBounceCompleted {
        source_track_id: TrackId,
        target_track_id: TrackId,
        clip: Option<BouncedClipData>,
    },
    /// A "bounce in place" run failed. The string is user-facing.
    TrackBounceError(String),
    /// "Bounce in place" run was cancelled by the user via
    /// `AudioCommand::CancelBounce`. Distinct from `TrackBounceError`
    /// so the app can drop the modal without surfacing a noisy error
    /// banner. The engine guarantees the freshly-added target track is
    /// removed before this event fires, so the app only has to clear
    /// its in-progress state.
    TrackBounceCancelled {
        target_track_id: TrackId,
    },
    /// Periodic progress update for a "bounce in place" run, emitted
    /// from both the offline (`to_audio_clip`) and realtime
    /// (`poll_pending_bounce`) paths so the app can drive a progress
    /// bar. `fraction` is in `[0.0, 1.0]`.
    BounceProgress {
        fraction: f32,
    },
    /// Response to `SaveClipsToProjectDir`: every in-engine audio
    /// clip has a `.wav` file on disk. The map is `clip_id` →
    /// project-relative path (e.g. `"audio/clip_42.wav"`), which
    /// the save path writes into `project.json`.
    ClipsSavedToProjectDir {
        clip_files: Vec<(ClipId, String)>,
    },
    /// All plugin states saved in batch.
    AllPluginStatesSaved {
        states: Vec<(PluginInstanceId, Vec<u8>)>,
    },
    /// Engine has been cleared of all state.
    AllCleared,

    // -- Instrument track events --
    InstrumentTrackAdded {
        track_id: TrackId,
    },

    // -- Vocal track events --
    VocalTrackAdded {
        track_id: TrackId,
    },

    // -- MIDI clip events --
    MidiClipCreated {
        clip_id: ClipId,
        track_id: TrackId,
        start_sample: SamplePos,
        duration_ticks: u64,
        name: String,
        notes: Vec<MidiNote>,
        trim_start_ticks: u64,
        trim_end_ticks: u64,
    },
    MidiClipMoved {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        new_track_id: TrackId,
    },
    MidiClipTrimmed {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        trim_start_ticks: u64,
        trim_end_ticks: u64,
    },
    MidiClipDeleted {
        clip_id: ClipId,
    },

    // -- MIDI note editing events --
    MidiNoteAdded {
        clip_id: ClipId,
        note: MidiNote,
    },
    MidiNoteRemoved {
        clip_id: ClipId,
        note_index: usize,
    },
    MidiNoteMoved {
        clip_id: ClipId,
        note_index: usize,
        new_start_tick: u64,
        new_note: u8,
    },
    MidiNoteResized {
        clip_id: ClipId,
        note_index: usize,
        new_duration_ticks: u64,
    },
    MidiNoteVelocitySet {
        clip_id: ClipId,
        note_index: usize,
        velocity: f32,
    },

    // -- Bulk MIDI note edits (quantize / humanize / groove) --
    /// One atomic bulk edit result: the full resulting note array for
    /// `clip_id` after a quantize / humanize / groove operation. The app
    /// mirrors this into `ClipState` and records the prior notes for a
    /// single-step undo. Note order is preserved (operations work by
    /// index and never reorder, merge, or drop notes).
    MidiNotesEdited {
        clip_id: ClipId,
        notes: Vec<MidiNote>,
    },
    /// A groove template extracted from a clip via
    /// `AudioCommand::ExtractGrooveFromClip`.
    GrooveExtracted {
        template: GrooveTemplate,
    },

    // -- Bus events --
    BusAdded {
        bus_id: BusId,
        name: String,
    },
    BusRemoved {
        bus_id: BusId,
    },
    BusPluginAdded {
        bus_id: BusId,
        instance_id: PluginInstanceId,
        plugin_name: String,
        clap_plugin_id: String,
        clap_file_path: String,
        params: Vec<ParamInfo>,
        has_gui: bool,
    },
    BusPluginRemoved {
        bus_id: BusId,
        instance_id: PluginInstanceId,
    },

    // -- Aux sends + return busses --
    /// A bus's return-role flag changed (see `AudioCommand::SetBusRole`).
    BusRoleChanged {
        bus_id: BusId,
        is_return: bool,
    },
    /// An aux send was created or updated. Carries the engine-resolved
    /// send so the app mirror matches engine state (including the
    /// allocated `send_id` and any clamping of `level_db`).
    AuxSendChanged {
        send_id: SendId,
        source: SendSource,
        dest: BusId,
        level_db: f32,
        pre_fader: bool,
        enabled: bool,
    },
    /// An aux send was removed (see `AudioCommand::RemoveAuxSend`).
    AuxSendRemoved {
        send_id: SendId,
    },
    /// An aux send was rejected and not registered. `reason` is a
    /// plain-language explanation suitable for surfacing in the UI
    /// (e.g. a self-route or a feedback cycle).
    AuxSendRejected {
        source: SendSource,
        dest: BusId,
        reason: String,
    },

    // -- Master FX events --
    MasterPluginAdded {
        instance_id: PluginInstanceId,
        plugin_name: String,
        clap_plugin_id: String,
        clap_file_path: String,
        params: Vec<ParamInfo>,
        has_gui: bool,
    },
    MasterPluginRemoved {
        instance_id: PluginInstanceId,
    },
    TrackFxBypassChanged {
        track_id: TrackId,
        bypassed: bool,
    },
    BusFxBypassChanged {
        bus_id: BusId,
        bypassed: bool,
    },
    MasterFxBypassChanged {
        bypassed: bool,
    },

    // -- Hardware MIDI I/O --
    /// Result of `AudioCommand::ListMidiInputDevices`.
    MidiInputDevicesListed {
        devices: Vec<MidiDeviceInfo>,
    },
    /// Result of `AudioCommand::ListMidiOutputDevices`.
    MidiOutputDevicesListed {
        devices: Vec<MidiDeviceInfo>,
    },
    // -- External-instrument tracks (doc #169, epic #39) --
    /// An external-instrument config was stored, replaced, or one of its
    /// fields changed (bank/program, latency offset). Carries the config
    /// exactly as the engine holds it so the app mirror matches engine state.
    ExternalInstrumentChanged {
        config: ExternalInstrument,
    },
    /// The external-instrument config for `track_id` was removed — the track
    /// is no longer an external instrument.
    ExternalInstrumentCleared {
        track_id: TrackId,
    },
    /// The external-instrument track's MIDI output device is offline: a patch
    /// send found no live connection, or a device re-check found it gone. The
    /// route is preserved (config untouched) so a replug reconnects. `device`
    /// is the configured MIDI output name, if any.
    ExternalInstrumentMidiOutOffline {
        track_id: TrackId,
        device: Option<String>,
    },
    /// The external-instrument track's audio-return input device is offline —
    /// a device re-check found it gone. The route is preserved so a replug
    /// reconnects. `device` is the configured return input name, if any.
    ExternalInstrumentReturnInputOffline {
        track_id: TrackId,
        device: Option<String>,
    },
    /// Result of `AudioCommand::DetectExternalInstrumentLatency`: the
    /// round-trip latency the engine measured for `track_id` by pinging the
    /// hardware (MIDI impulse out → audio return in). `latency_samples` is the
    /// measured round-trip at the engine sample rate; `latency_ms` is the same
    /// value in milliseconds for display. The engine has already applied this
    /// as the track's effective offset (the manual offset is the floor, so the
    /// applied value is `max(manual_offset, latency_samples)`) and the app
    /// mirror updates its displayed/applied offset to match.
    ExternalInstrumentLatencyMeasured {
        track_id: TrackId,
        latency_samples: i64,
        latency_ms: f32,
    },
    /// `AudioCommand::DetectExternalInstrumentLatency` could not measure a
    /// round-trip for `track_id` — the MIDI output was offline, the audio
    /// return delivered no/silent frames, or no impulse returned within the
    /// listen window. Nothing is changed (the existing offset stands); the app
    /// surfaces `reason` instead of leaving the user waiting on a hung ping.
    ExternalInstrumentLatencyDetectFailed {
        track_id: TrackId,
        reason: String,
    },

    /// Incoming MIDI Clock Start (0xFA) — external master started its
    /// transport and the engine is now playing in sync with it.
    MidiClockStarted,
    /// Incoming MIDI Clock Continue (0xFB) — external master resumed.
    MidiClockContinued,
    /// Incoming MIDI Clock Stop (0xFC) — external master stopped.
    MidiClockStopped,
    /// External master tempo derived from incoming clock pulses, in
    /// BPM. Sent at most a few times per second once the input clock
    /// stabilises so the GUI BPM display can mirror the master.
    MidiClockTempoDetected {
        bpm: f32,
    },

    // -- Freeze events --
    /// Periodic progress update for a freeze render, emitted as the
    /// offline renderer processes chunks. `fraction` is in `[0.0, 1.0]`.
    /// Mirrors the bounce progress event shape.
    FreezeProgress {
        track_id: TrackId,
        fraction: f32,
    },
    /// Freeze render completed successfully. The track now has a valid
    /// freeze cache that can be attached via `SetTrackFrozenSource`.
    FreezeCompleted {
        track_id: TrackId,
        cache_ref: FreezeCacheRef,
    },
    /// Freeze render failed. The string is user-facing.
    FreezeError {
        track_id: TrackId,
        message: String,
    },
    /// Freeze render was cancelled by the user via `AudioCommand::CancelFreeze`.
    /// The engine guarantees the partially-written cache file (if any) is
    /// removed before this event fires.
    FreezeCancelled {
        track_id: TrackId,
    },

    /// Snapshot of peak meters for VU display, sent in response to
    /// `AudioCommand::PollPeaks`. Replaces the older `read_and_clear_peaks`
    /// getter so the GUI never reaches into engine state directly.
    PeakSnapshot {
        track_peaks: Vec<(TrackId, f32, f32)>,
        bus_peaks: Vec<(BusId, f32, f32)>,
        master_peak_l: f32,
        master_peak_r: f32,
    },

    // -- Reference track (A/B) events --
    /// Progress of the offline analysis a freshly-loaded reference goes
    /// through. Emitted in stage order before the final
    /// `ReferenceLoaded`.
    ReferenceAnalysisProgress {
        id: ReferenceId,
        stage: ReferenceAnalysisStage,
    },
    /// A reference track finished loading and analysing. `path` is the
    /// source file it was loaded from; `integrated_lufs` is its measured
    /// loudness (used for loudness matching); `waveform_peaks` is the
    /// downsampled overview (min, max) per chunk of frames;
    /// `length_samples` is the reference's total length in frames, so the
    /// panel can map its playback cursor / markers onto the overview.
    ReferenceLoaded {
        id: ReferenceId,
        name: String,
        path: String,
        integrated_lufs: f32,
        waveform_peaks: Vec<(f32, f32)>,
        length_samples: u64,
    },
    /// A reference track failed to load (decode error, missing file, …).
    /// `reason` is user-facing.
    ReferenceLoadFailed {
        path: String,
        reason: String,
    },
    /// A reference track was removed.
    ReferenceRemoved {
        id: ReferenceId,
    },
    /// The active reference selection changed.
    ActiveReferenceChanged {
        id: ReferenceId,
    },
    /// The monitored A/B source switched between mix and reference.
    ABSourceChanged {
        source: ABSource,
    },
    /// Loudness-match toggled. `offset_db` is the gain offset the engine
    /// applies to the active reference when `enabled` is true.
    RefLoudnessMatchChanged {
        enabled: bool,
        offset_db: f32,
    },
    /// The reference's manual level trim changed (dB).
    RefTrimChanged {
        db: f32,
    },
    /// A comparison marker was added to a reference. `marker_id` is the
    /// engine-allocated id.
    RefMarkerAdded {
        ref_id: ReferenceId,
        marker_id: u32,
        position_samples: SamplePos,
        label: String,
    },
    /// A comparison marker was removed from a reference.
    RefMarkerRemoved {
        ref_id: ReferenceId,
        marker_id: u32,
    },
    /// A reference's own playback cursor moved.
    RefPositionChanged {
        ref_id: ReferenceId,
        position_samples: SamplePos,
    },
    /// The reference loop-to-mix follow mode toggled.
    RefLoopToMixChanged {
        enabled: bool,
    },
    /// A/B meter snapshot in response to `AudioCommand::PollABMeters`.
    /// `reference` is `None` when no reference is active.
    ABMeterSnapshot {
        mix: MeterSnapshot,
        reference: Option<MeterSnapshot>,
    },

    // -- MIDI Learn & hardware controller mapping (doc #167 §2 E2) --
    /// In MIDI Learn mode, the first qualifying control-surface message
    /// arrived: report the armed `target` together with the captured
    /// `source` so the app can create / replace the binding (with default
    /// range / mode) and exit learn mode. The engine leaves learn mode after
    /// emitting this.
    MidiLearnCaptured {
        target: MidiTarget,
        source: ControlSource,
    },
    /// A binding was inserted or replaced in the engine's active set (echo of
    /// `SetMidiBinding`, or one per binding of a `SetControllerMap`). Lets the
    /// app rebuild `MidiMapState` purely from events, including after
    /// project-load replay.
    MidiBindingChanged {
        binding: MidiBinding,
    },
    /// A binding was removed from the active set (echo of `ClearMidiBinding`,
    /// or one per cleared binding of `ClearAllMidiBindings`).
    MidiBindingCleared {
        id: BindingId,
    },
    /// Throttled control-rate feedback that a hardware move drove `target` to
    /// `value_norm` (normalized 0..=1), so the app can update the on-screen
    /// fader/knob/toggle without round-tripping through the normal value path.
    ControlSurfaceParamChanged {
        target: MidiTarget,
        value_norm: f32,
    },
    /// The set of available control-surface MIDI input device names changed
    /// (hot-plug, or initial enumeration), so the app can refresh its device
    /// picker.
    ControlSurfaceDevicesChanged {
        inputs: Vec<String>,
    },

    // -- Audition preview (doc #175) --
    /// Throttled (control-rate, ~60 Hz) audition playhead position in source
    /// frames, so the GUI can draw a scrub playhead over the preview. Only
    /// emitted while a preview is playing.
    AuditionPosition {
        frame: u64,
    },
    /// The audition preview stopped — either it reached the end of a
    /// non-looping file, or it was stopped via `AudioCommand::StopAudition`.
    AuditionStopped,
}
