//! GUI → engine command enum.
use std::path::PathBuf;

use super::{
    ABSource, BusId, ClipId, FadeCurve, MidiNote, PluginInstanceId, ReferenceId, SamplePos,
    SignaturePoint, TempoPoint, TrackId, TrackOutput,
};

/// Commands sent from the GUI to the audio engine.
#[derive(Debug, Clone)]
pub enum AudioCommand {
    Play,
    /// Start recording on every armed track. When `precount_bars > 0`,
    /// the engine rewinds the playhead by that many bars, force-enables
    /// the metronome, and begins playback; the input stream opens and
    /// `recording` flips true once the playhead catches up to the
    /// original position.
    Record {
        precount_bars: u8,
    },
    Pause,
    Stop,
    SeekTo(SamplePos),
    ImportClip {
        track_id: TrackId,
        path: String,
        start_sample: SamplePos,
    },
    /// Import one or more source files into the project pool **without**
    /// placing a clip. Each file is decoded, channel up/down-mixed and
    /// resampled to the project rate, copied into `{project_dir}/audio/`
    /// under a stable `asset_{id}.wav` name, and has its waveform peaks
    /// computed — all on a worker thread. Per file the engine emits an
    /// ordered `ImportProgress` lifecycle (`Queued` → `Working` →
    /// `Done`) plus a final `AssetImported` on success, or `ImportFailed`
    /// on error. Requires a project directory (set via
    /// [`AudioCommand::SetProjectDir`]); decoupled from clip placement,
    /// so it needs no `track_id`.
    ImportAudioToPool {
        paths: Vec<String>,
    },
    MoveClip {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        new_track_id: TrackId,
    },
    TrimClip {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        trim_start_frames: u64,
        trim_end_frames: u64,
    },
    DeleteClip {
        clip_id: ClipId,
    },
    /// Set the fade-in/out lengths and curves of an audio clip. The
    /// engine clamps each fade length to the clip's visible duration and
    /// emits `AudioEvent::ClipFadeChanged` with the clamped values.
    SetClipFade {
        clip_id: ClipId,
        fade_in_frames: u64,
        fade_in_curve: FadeCurve,
        fade_out_frames: u64,
        fade_out_curve: FadeCurve,
    },
    /// Set the per-clip gain of an audio clip in decibels. The engine
    /// clamps the value to a sane range and emits
    /// `AudioEvent::ClipGainChanged` with the clamped value.
    SetClipGain {
        clip_id: ClipId,
        gain_db: f32,
    },
    SetTrackVolume {
        track_id: TrackId,
        volume: f32,
    },
    SetTrackPan {
        track_id: TrackId,
        pan: f32,
    },
    SetTrackMute {
        track_id: TrackId,
        muted: bool,
    },
    SetMasterVolume {
        volume: f32,
    },
    SetTrackSolo {
        track_id: TrackId,
        soloed: bool,
    },
    /// Add an audio track. When `id_hint`/`name` are provided (e.g. by
    /// project load), the engine honours them and bumps its allocator
    /// past the hinted id so subsequent fresh tracks don't collide.
    AddTrack {
        id_hint: Option<TrackId>,
        name: Option<String>,
    },
    RemoveTrack {
        track_id: TrackId,
    },
    /// Register an app-side sub-track with the audio engine so the mixer
    /// can drive it from its parent plugin's output port. The app
    /// allocates the id itself (high range, never colliding with
    /// engine ids counting up from 1) and passes it here. Called after
    /// `AudioEvent::PluginAdded` for every non-main output port on a
    /// multi-output plugin.
    CreateSubTrack {
        sub_id: TrackId,
        parent_track_id: TrackId,
        output_port_index: u32,
        name: String,
    },
    SetTrackRecordArm {
        track_id: TrackId,
        armed: bool,
    },
    SetTrackMonitor {
        track_id: TrackId,
        enabled: bool,
    },
    SetTrackMono {
        track_id: TrackId,
        mono: bool,
    },
    SetTrackInputDevice {
        track_id: TrackId,
        device_name: Option<String>,
    },
    /// Set the 0-indexed starting input channel for a track. Mono
    /// tracks capture just this channel; stereo tracks capture this
    /// channel as L and `port_index + 1` as R.
    SetTrackInputPort {
        track_id: TrackId,
        port_index: u16,
    },
    ListInputDevices,
    SetBpm {
        bpm: f32,
    },
    /// Send the full tempo event list so the engine can compute BPM
    /// at any playhead position internally, without per-tick updates.
    SetTempoEvents {
        tempo: Vec<TempoPoint>,
        signature: Vec<SignaturePoint>,
    },
    SetTimeSignature {
        numerator: u8,
        denominator: u8,
    },
    SetMetronomeEnabled {
        enabled: bool,
    },
    AddPlugin {
        track_id: TrackId,
        clap_file_path: String,
        clap_plugin_id: String,
        /// If provided, the engine uses this id instead of allocating a
        /// new one and bumps its allocator past it. Set by project load.
        id_hint: Option<PluginInstanceId>,
    },
    RemovePlugin {
        track_id: TrackId,
        instance_id: PluginInstanceId,
    },
    ScanPlugins,
    SetPluginParam {
        instance_id: PluginInstanceId,
        param_id: u32,
        value: f64,
    },
    /// Set loop (cycle) range. When enabled, playback wraps from loop_out back to
    /// loop_in, and any recording is trimmed to [loop_in, loop_out]. If
    /// loop_out <= loop_in, no clip is produced.
    SetLoopRange {
        enabled: bool,
        loop_in: SamplePos,
        loop_out: SamplePos,
    },
    SavePluginState {
        instance_id: PluginInstanceId,
    },
    LoadPluginState {
        instance_id: PluginInstanceId,
        data: Vec<u8>,
    },
    /// Open the plugin's editor window (requires CLAP_EXT_GUI).
    OpenPluginEditor {
        instance_id: PluginInstanceId,
    },
    /// Close the plugin's editor window.
    ClosePluginEditor {
        instance_id: PluginInstanceId,
    },
    /// Offline render of the project to a WAV file.
    BounceToWav {
        path: String,
    },
    /// Bounce in place — render one instrument track (and any of its
    /// sub-tracks) to a single in-RAM stereo `AudioClip` on
    /// `target_track_id`. The app pre-creates the audio track via
    /// [`AudioCommand::AddTrack`] with `id_hint = Some(target_track_id)`
    /// and pre-allocates `target_clip_id` (same allocator pool as
    /// `LoadMidiClipDirect`). Excludes master FX / master volume so the
    /// captured PCM plays back through master once on subsequent
    /// playback. Used for instrument tracks driven by an internal synth
    /// plugin; tracks that drive an external MIDI device need a real-
    /// time bounce that is not implemented by this command.
    BounceTrackToAudio {
        source_track_id: TrackId,
        target_track_id: TrackId,
        target_clip_id: ClipId,
        name: String,
    },
    /// Real-time "bounce in place" for instrument tracks driven by an
    /// external MIDI device. The engine snapshots every other track's
    /// mute state, mutes them all so only the source's MIDI fires to
    /// hardware, configures `target_track_id`'s audio input + record
    /// arm, seeks to the source's first MIDI start, and runs the
    /// transport from there to the last MIDI end + 2 s tail. When the
    /// playhead crosses the end, the engine pauses, finalizes the
    /// recording (emits `RecordingFinished`), restores the mute snapshot
    /// and mutes the source, then emits `TrackBounceCompleted`.
    BounceTrackRealtimeToAudio {
        source_track_id: TrackId,
        target_track_id: TrackId,
        input_device_name: String,
        input_port_index: u16,
        /// Capture as mono (one channel duplicated to L/R) vs stereo
        /// (two consecutive channels). External instruments returning a
        /// stereo pair want `false`; a single guitar/voice mic wants `true`.
        mono: bool,
    },
    /// Cancel an in-flight bounce-in-place run. Aborts the offline
    /// renderer between chunks (via a shared atomic the renderer
    /// polls), or pauses the transport + restores mute state for the
    /// realtime path. In both cases the freshly-added target track is
    /// removed and a `TrackBounceCancelled` event is emitted.
    CancelBounce,
    /// Set the current project directory. Recorded and imported
    /// clips are written into `{project_dir}/audio/` as WAV files,
    /// and recording refuses to start if no project directory has
    /// been set. Sent by the app whenever a project is opened,
    /// created, or saved-as to a new location.
    SetProjectDir(PathBuf),
    /// Load an audio clip from a WAV file on disk (project load
    /// path). The engine memory-maps the file and references it
    /// via `ClipSource::Mapped`, so the PCM data never materialises
    /// as a contiguous in-RAM buffer.
    LoadClipFromWav {
        clip_id: ClipId,
        track_id: TrackId,
        start_sample: SamplePos,
        path: PathBuf,
        name: String,
        trim_start_frames: u64,
        trim_end_frames: u64,
    },
    /// Ensure every in-engine audio clip has a WAV file on disk at
    /// `{project_dir}/audio/clip_{id}.wav`. Recorded clips already
    /// do (they stream there during capture); in-RAM imported clips
    /// get transcoded. Emits `AudioEvent::ClipsSavedToProjectDir`
    /// when done so the save path can write project.json.
    SaveClipsToProjectDir,
    /// Batch save all plugin states for project save.
    SaveAllPluginStates,
    /// Remove all tracks, clips, and plugins (for project load).
    ClearAll,

    // -- Instrument track commands --
    /// Add an instrument track. See [`AudioCommand::AddTrack`] for how
    /// `id_hint`/`name` are honoured.
    AddInstrumentTrack {
        id_hint: Option<TrackId>,
        name: Option<String>,
    },

    // -- Vocal track commands --
    /// Add a vocal track. Engine-side this is an instrument-shaped track
    /// (accepts live MIDI) but its playback path runs through the audio
    /// clip pipeline so the SVS-rendered WAV is what's heard.
    AddVocalTrack {
        id_hint: Option<TrackId>,
        name: Option<String>,
    },

    // -- MIDI clip commands --
    CreateMidiClip {
        track_id: TrackId,
        start_sample: SamplePos,
        duration_ticks: u64,
        name: String,
    },
    LoadMidiClipDirect {
        clip_id: ClipId,
        track_id: TrackId,
        start_sample: SamplePos,
        duration_ticks: u64,
        notes: Vec<MidiNote>,
        name: String,
        trim_start_ticks: u64,
        trim_end_ticks: u64,
    },
    MoveMidiClip {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        new_track_id: TrackId,
    },
    TrimMidiClip {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        trim_start_ticks: u64,
        trim_end_ticks: u64,
    },
    DeleteMidiClip {
        clip_id: ClipId,
    },

    // -- MIDI note editing commands --
    AddMidiNote {
        clip_id: ClipId,
        note: MidiNote,
    },
    RemoveMidiNote {
        clip_id: ClipId,
        note_index: usize,
    },
    MoveMidiNote {
        clip_id: ClipId,
        note_index: usize,
        new_start_tick: u64,
        new_note: u8,
    },
    ResizeMidiNote {
        clip_id: ClipId,
        note_index: usize,
        new_duration_ticks: u64,
    },
    SetMidiNoteVelocity {
        clip_id: ClipId,
        note_index: usize,
        velocity: f32,
    },

    // -- Live MIDI input --
    SendNoteOn {
        track_id: TrackId,
        note: u8,
        velocity: f32,
    },
    SendNoteOff {
        track_id: TrackId,
        note: u8,
    },

    // -- Hardware MIDI I/O --
    /// Enumerate hardware MIDI input ports and emit
    /// `AudioEvent::MidiInputDevicesListed`.
    ListMidiInputDevices,
    /// Enumerate hardware MIDI output ports and emit
    /// `AudioEvent::MidiOutputDevicesListed`.
    ListMidiOutputDevices,
    /// Set the hardware MIDI input device for an instrument track.
    /// Notes received from the device are routed to the track's
    /// instrument plugin and (when armed) recorded into a MIDI clip.
    /// `device = None` disconnects.
    SetTrackMidiInput {
        track_id: TrackId,
        device: Option<String>,
        /// 0-indexed channel filter (0..=15), or `None` for omni.
        channel: Option<u8>,
    },
    /// Set the hardware MIDI output device for an instrument track.
    /// Notes played by the track (timeline + live input) are also
    /// sent to this device — the instrument plugin still plays.
    /// `device = None` disconnects.
    SetTrackMidiOutput {
        track_id: TrackId,
        device: Option<String>,
        /// 0-indexed channel (0..=15) the output uses. `None` = channel 1.
        channel: Option<u8>,
    },

    /// Configure the global MIDI clock master (Resonance → device).
    /// When `enabled` is true and `device` is set, the engine emits
    /// 24-PPQN clock pulses plus Start/Stop/Continue/Song Position
    /// messages aligned to the project tempo and transport.
    SetMidiClockOutput {
        device: Option<String>,
        enabled: bool,
    },
    /// Configure the global MIDI clock slave (device → Resonance).
    /// When enabled, incoming Start/Continue/Stop messages drive
    /// transport and clock pulses smooth the project BPM toward the
    /// external master.
    SetMidiClockInput {
        device: Option<String>,
        enabled: bool,
    },

    // -- Bus commands --
    /// Add a bus. See [`AudioCommand::AddTrack`] for how `id_hint`/`name`
    /// are honoured.
    AddBus {
        id_hint: Option<BusId>,
        name: Option<String>,
    },
    RemoveBus {
        bus_id: BusId,
    },
    SetBusVolume {
        bus_id: BusId,
        volume: f32,
    },
    SetBusPan {
        bus_id: BusId,
        pan: f32,
    },
    SetBusMute {
        bus_id: BusId,
        muted: bool,
    },
    SetBusName {
        bus_id: BusId,
        name: String,
    },
    SetTrackOutput {
        track_id: TrackId,
        output: TrackOutput,
    },
    AddPluginToBus {
        bus_id: BusId,
        clap_file_path: String,
        clap_plugin_id: String,
        /// If provided, the engine uses this id instead of allocating a
        /// new one and bumps its allocator past it. Set by project load.
        id_hint: Option<PluginInstanceId>,
    },
    RemovePluginFromBus {
        bus_id: BusId,
        instance_id: PluginInstanceId,
    },

    // -- Master FX chain + bypass --
    /// Add a plugin to the master bus insert chain. Master FX run after
    /// every track/bus has been summed, before the master volume pass.
    AddPluginToMaster {
        clap_file_path: String,
        clap_plugin_id: String,
        /// If provided, the engine uses this id instead of allocating a
        /// new one and bumps its allocator past it. Set by project load.
        id_hint: Option<PluginInstanceId>,
    },
    RemovePluginFromMaster {
        instance_id: PluginInstanceId,
    },
    /// Bypass every effect plugin on a track. Instrument plugins
    /// (slot 0 on instrument tracks) keep running.
    SetTrackFxBypass {
        track_id: TrackId,
        bypassed: bool,
    },
    SetBusFxBypass {
        bus_id: BusId,
        bypassed: bool,
    },
    SetMasterFxBypass {
        bypassed: bool,
    },

    /// Ask the engine to snapshot and clear every peak meter (per-track,
    /// per-bus, master L/R) and reply with `AudioEvent::PeakSnapshot`.
    /// Driven by the GUI's per-frame VU update; replaces the older
    /// direct getter that contended with the mixer's RwLocks.
    PollPeaks,

    // -- Reference track (A/B) commands --
    /// Load an external reference track from disk for A/B comparison.
    /// The engine decodes it on a worker, measures its integrated
    /// loudness and waveform overview, and emits
    /// `AudioEvent::ReferenceLoaded` (with intermediate
    /// `ReferenceAnalysisProgress`) or `ReferenceLoadFailed`. When
    /// `id_hint` is provided (e.g. project load) the engine honours it
    /// and bumps its allocator past it; otherwise it allocates a fresh
    /// [`ReferenceId`].
    LoadReferenceTrack {
        id_hint: Option<ReferenceId>,
        path: PathBuf,
    },
    /// Remove a loaded reference track and free its decoded PCM. Emits
    /// `AudioEvent::ReferenceRemoved`. If it was the active reference,
    /// the engine also clears the active selection.
    RemoveReferenceTrack {
        id: ReferenceId,
    },
    /// Select which loaded reference the A/B monitor auditions. Emits
    /// `AudioEvent::ActiveReferenceChanged`.
    SetActiveReference {
        id: ReferenceId,
    },
    /// Switch the monitored signal between the project mix and the
    /// active reference. Emits `AudioEvent::ABSourceChanged`.
    SetABSource {
        source: ABSource,
    },
    /// Toggle loudness-matching the active reference to the mix. When
    /// enabled the engine applies the measured per-reference gain
    /// offset so both audition at the same loudness. Emits
    /// `AudioEvent::RefLoudnessMatchChanged` (carrying the applied
    /// offset).
    SetRefLoudnessMatch {
        enabled: bool,
    },
    /// Manual level trim (dB) applied to the reference on top of any
    /// loudness match. Emits `AudioEvent::RefTrimChanged`.
    SetRefTrim {
        db: f32,
    },
    /// Add a comparison marker to a reference at a sample position.
    /// The engine allocates the marker id and emits
    /// `AudioEvent::RefMarkerAdded`.
    AddRefMarker {
        ref_id: ReferenceId,
        position_samples: SamplePos,
        label: String,
    },
    /// Remove a comparison marker from a reference. Emits
    /// `AudioEvent::RefMarkerRemoved`.
    RemoveRefMarker {
        ref_id: ReferenceId,
        marker_id: u32,
    },
    /// Seek the reference's own playback cursor to a sample position.
    /// Emits `AudioEvent::RefPositionChanged`.
    SetRefPosition {
        ref_id: ReferenceId,
        position_samples: SamplePos,
    },
    /// Toggle whether the reference's playback cursor follows the mix
    /// transport (loop-to-mix) or plays from its own cursor. Emits
    /// `AudioEvent::RefLoopToMixChanged`.
    SetRefLoopToMix {
        enabled: bool,
    },
    /// Ask the engine for a fresh A/B meter snapshot (mix plus the
    /// active reference) and reply with `AudioEvent::ABMeterSnapshot`.
    /// Driven by the GUI's per-frame meter update.
    PollABMeters,

    /// Break the engine-thread loop and let the thread exit cleanly.
    /// Required because the engine thread holds its own `Sender` clone
    /// (`cmd_tx_retry`) for the retry path, which prevents the channel
    /// from ever returning `Disconnected` even after every external
    /// sender has dropped. Sent by `AudioEngine::shutdown` / `Drop`.
    ShutDown,
}
