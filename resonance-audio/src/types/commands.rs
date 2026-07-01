//! GUI → engine command enum.
use std::path::PathBuf;
use std::sync::Arc;

use resonance_common::{BindingId, ControllerMap, MidiBinding, MidiTarget};

use resonance_common::{AutomationLane, AutomationTarget};

use super::{
    ABSource, BusId, ClipId, ExportSettings, FadeCurve, FrozenSource, MidiNote, PluginInstanceId,
    ReferenceId, SamplePos, SendId, SendSource, SignaturePoint, StemBitDepth, StemTarget,
    TempoPoint, TrackId, TrackOutput, WarpAlgorithm, WarpMarker,
};
use crate::quantize::{Division, GrooveTemplate, QuantizeMode};

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
    /// Set an audio clip's warp ("follow tempo") parameters. The engine
    /// stores them on the clip and emits `AudioEvent::ClipWarpChanged`
    /// with the stored values. Warp markers are carried separately by
    /// [`AudioCommand::SetClipWarpMarkers`]. Defaults (`warp_enabled =
    /// false`, `original_bpm = None`, `transpose_semitones = 0.0`) leave
    /// the clip reading its source 1:1.
    SetClipWarp {
        clip_id: ClipId,
        warp_enabled: bool,
        original_bpm: Option<f32>,
        transpose_semitones: f32,
        warp_algorithm: WarpAlgorithm,
    },
    /// Replace an audio clip's full warp-marker set. Adding, moving and
    /// removing a marker are all expressed as a full-set replace. The
    /// engine sorts the incoming markers by `timeline_beat` to uphold the
    /// [`WarpMarker`] sorted invariant, stores them, and emits
    /// `AudioEvent::ClipWarpMarkersChanged` with the sorted set.
    SetClipWarpMarkers {
        clip_id: ClipId,
        markers: Vec<WarpMarker>,
    },
    /// Run tempo/BPM detection over a clip's source audio. The engine
    /// runs the DSP detector and replies with
    /// `AudioEvent::ClipTempoDetected`. The detector and its reply event
    /// are wired up in a later todo; this command is plumbed here so the
    /// command/event boundary is complete.
    DetectClipTempo {
        clip_id: ClipId,
    },
    /// Run vocal pitch analysis (monophonic f0 detection + note
    /// segmentation) on the clip's mono mix off the realtime thread. The
    /// result is stored in the clip's [`VocalTuning`](super::VocalTuning)
    /// analysis cache and emitted as `AudioEvent::ClipPitchDetected`. A
    /// no-op (no event) when the clip no longer exists.
    AnalyzeClipPitch {
        clip_id: ClipId,
    },
    /// Store or replace the automation lane for its target. The engine
    /// holds one lane per [`AutomationTarget`]; sending a lane whose
    /// `target` already has an entry replaces it wholesale. The engine
    /// keeps the breakpoints sorted and echoes the stored lane back via
    /// `AudioEvent::AutomationLaneChanged`. No audio is applied yet — a
    /// later step samples the lane per block.
    SetAutomationLane {
        lane: AutomationLane,
    },
    /// Remove the automation lane stored for `target`. When a lane was
    /// present the engine emits `AudioEvent::AutomationLaneCleared`;
    /// clearing an absent target is a silent no-op.
    ClearAutomationLane {
        target: AutomationTarget,
    },
    /// Toggle a lane's "read" flag (`AutomationLane::enabled`) without
    /// replacing its breakpoints. The engine echoes the updated lane via
    /// `AudioEvent::AutomationLaneChanged`; toggling an absent target is
    /// a silent no-op.
    SetAutomationReadEnabled {
        target: AutomationTarget,
        enabled: bool,
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
    /// Toggle loop-record (cycle-record) capture. When enabled and the
    /// transport loops while a track is armed, the engine finalizes the
    /// in-progress capture into a distinct take at each loop seam and
    /// starts a fresh capture for the next pass, emitting
    /// `AudioEvent::TakeCaptured` per pass. When disabled, a looped
    /// recording keeps the legacy single-clip behaviour.
    SetLoopRecordMode(bool),
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
    /// Offline render of the project to a WAV file. Legacy entry point,
    /// kept as a thin shim: the engine maps it onto [`AudioCommand::ExportAudio`]
    /// with [`ExportSettings::default_wav`] (32-bit-float WAV at the engine
    /// rate) so existing callers keep working until the app migrates.
    BounceToWav {
        path: String,
    },
    /// Offline render + encode of the project to `path` using the
    /// format / loudness-normalization / metadata described by `settings`
    /// (see doc #196). Generalizes [`AudioCommand::BounceToWav`]. The
    /// WAV f32 path renders identically to the legacy bounce; other
    /// formats and the normalization passes land with the encoder-sink
    /// follow-up todos. Emits the `Export*` lifecycle events.
    ExportAudio {
        path: String,
        settings: ExportSettings,
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
    /// Offline "export stems": render several mix slices (one track, one
    /// bus, or the whole master) to separate WAV files. Every target is
    /// rendered over ONE shared range so the stems share a zero origin
    /// and re-import sample-aligned. Targets are rendered sequentially on
    /// a worker thread (like [`AudioCommand::BounceToWav`]); the engine
    /// emits `StemExportProgress` / `StemExportTargetDone` per target,
    /// `StemExportTargetError` for a target that fails to render or write
    /// (already-written stems are kept and the queue continues), and a
    /// final `StemExportComplete` listing the files actually written.
    ExportStems {
        /// The mix slices to render and where to write each one.
        targets: Vec<StemTarget>,
        /// Shared render window in engine samples. `None` renders the
        /// full project range (every audio + MIDI clip), matching the
        /// project bounce.
        range: Option<(SamplePos, SamplePos)>,
        /// Output WAV sample rate. The engine renders at its native rate
        /// and resamples on write only when this differs.
        sample_rate: u32,
        /// Output WAV bit depth / encoding.
        bit_depth: StemBitDepth,
        /// Render a tail past the end of the range so reverb / delay
        /// tails decay into the stem instead of being cut off.
        include_fx_tail: bool,
    },
    /// Cancel an in-flight stem export between targets. The worker polls
    /// a shared atomic and stops before the next target; stems already
    /// written stay on disk and a `StemExportCancelled` event reports
    /// them. Shares the bounce cancel flag, so it also aborts an offline
    /// bounce in progress (the two never overlap in practice).
    CancelStemExport,
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

    // -- Bulk MIDI note edits (quantize / humanize / groove) --
    /// Quantize the notes at `indices` in `clip_id` toward `grid`, using
    /// the engine's authoritative tempo map. Applied atomically; emits a
    /// single `AudioEvent::MidiNotesEdited` carrying the full resulting
    /// note array.
    QuantizeMidiNotes {
        clip_id: ClipId,
        /// Selected note indices to quantize; out-of-range indices are
        /// ignored and an empty selection is a no-op.
        indices: Vec<usize>,
        grid: Division,
        /// Blend toward the grid in `0.0..=1.0` (`1.0` snaps exactly).
        strength: f32,
        /// Swing applied to odd grid steps, `0.0..=1.0`.
        swing: f32,
        mode: QuantizeMode,
        /// Snap note-offs to the grid as well as note-ons.
        quantize_ends: bool,
        /// Apply the strength blend repeatedly (soft/iterative quantize).
        iterative: bool,
    },
    /// Apply seeded timing + velocity jitter to the notes at `indices`.
    /// Deterministic for a given `seed`; emits one `MidiNotesEdited`.
    HumanizeMidiNotes {
        clip_id: ClipId,
        indices: Vec<usize>,
        /// Maximum absolute timing offset in ticks.
        timing_ticks: u32,
        /// Velocity jitter fraction, `0.0..=1.0`.
        vel_amt: f32,
        seed: u64,
    },
    /// Apply a groove template to the notes at `indices` at `strength`.
    /// Emits one `MidiNotesEdited`.
    ApplyGrooveToClip {
        clip_id: ClipId,
        indices: Vec<usize>,
        template: GrooveTemplate,
        /// Template blend, `0.0..=1.0`.
        strength: f32,
    },
    /// Extract a groove template from `clip_id` at `grid` resolution.
    /// Reads the clip but does not modify it; emits
    /// `AudioEvent::GrooveExtracted` with the derived template.
    ExtractGrooveFromClip {
        clip_id: ClipId,
        grid: Division,
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

    // -- External-instrument tracks (doc #169, epic #39) --
    /// Mark a track as an external instrument (or replace its config). The
    /// MIDI output device/channel and audio-return device/channels are set
    /// through the normal `SetTrackMidiOutput` / `SetTrackInputDevice` /
    /// `SetTrackInputPort` commands; this carries only the bank/program and
    /// latency offset that have no home on a plain track. The engine echoes
    /// the stored config via `AudioEvent::ExternalInstrumentChanged`.
    SetExternalInstrument {
        config: resonance_common::ExternalInstrument,
    },
    /// Take a track out of external-instrument mode, dropping its config. The
    /// engine emits `AudioEvent::ExternalInstrumentCleared` when a config was
    /// present; clearing a non-external track is a silent no-op.
    ClearExternalInstrument {
        track_id: TrackId,
    },
    /// Set the selected bank/program for an external-instrument track and fire
    /// the patch send (Bank Select + Program Change) on the track's MIDI output
    /// channel. The engine echoes the updated config via
    /// `ExternalInstrumentChanged`; if the MIDI output is offline it also emits
    /// `ExternalInstrumentMidiOutOffline` while preserving the route. No-op when
    /// the track is not an external instrument.
    SetExternalInstrumentPatch {
        track_id: TrackId,
        /// Combined 14-bit bank (MSB << 7 | LSB), or `None` to send no Bank
        /// Select.
        bank: Option<u16>,
        /// Program number (`0..=127`), or `None` to send no Program Change.
        program: Option<u8>,
    },
    /// Set the manual latency offset (samples) for an external-instrument
    /// track. The engine echoes the updated config via
    /// `ExternalInstrumentChanged`. No-op when the track is not an external
    /// instrument.
    SetExternalInstrumentLatencyOffset {
        track_id: TrackId,
        latency_offset_samples: i64,
    },
    /// Re-check an external-instrument track's MIDI output and audio-return
    /// devices against the currently-available hardware and report any that
    /// have gone offline (`ExternalInstrumentMidiOutOffline` /
    /// `ExternalInstrumentReturnInputOffline`). The config is preserved so a
    /// replug reconnects. No-op when the track is not an external instrument.
    CheckExternalInstrumentDevices {
        track_id: TrackId,
    },
    /// Re-send Bank Select + Program Change for **every** external-instrument
    /// track from its stored config, without mutating any config. Sent by the
    /// app once after a project load has replayed all `SetExternalInstrument`
    /// configs, so a freshly-powered synth lands on its saved patch; the engine
    /// also fires this itself at transport start. Tracks with no bank/program
    /// are skipped; an offline output is reported per track via
    /// `ExternalInstrumentMidiOutOffline` while its route is preserved.
    ResendExternalInstrumentPatches,
    /// Auto-detect the round-trip latency of an external-instrument track:
    /// open its audio-return input, fire a short impulse note out its MIDI
    /// output, and time how long the return takes to come back. The result
    /// is reported via `AudioEvent::ExternalInstrumentLatencyMeasured`
    /// (samples + ms), and the engine applies it as the track's offset
    /// (raising the manual offset, which stays the floor) and republishes the
    /// plugin-delay-compensation table. Transport must be stopped. If the
    /// return can't be detected (no/silent input, MIDI output offline) the
    /// engine emits `ExternalInstrumentLatencyDetectFailed` with a reason and
    /// changes nothing. No-op when the track is not an external instrument.
    DetectExternalInstrumentLatency {
        track_id: TrackId,
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

    // -- Aux sends + return busses --
    /// Mark a bus as an aux *return* bus (or clear the flag). Emits
    /// `AudioEvent::BusRoleChanged`. No-op if the bus does not exist.
    SetBusRole {
        bus_id: BusId,
        is_return: bool,
    },
    /// Create or update (upsert) an aux send from a track or bus into a
    /// return bus. When `id_hint` is `None` the engine allocates a fresh
    /// `SendId`; when `Some(id)` it updates the existing send (or honours
    /// the id for a fresh send on project load, bumping its allocator
    /// past it). Covers create / re-route / level / pre-post / enable in
    /// one command. The engine runs cyclic-route validation before
    /// registering: a send routing a bus to itself, or to a destination
    /// whose own sends already reach the source bus, is rejected with
    /// `AudioEvent::AuxSendRejected` and not stored. On success the
    /// engine emits `AudioEvent::AuxSendChanged` with the resolved send.
    SetAuxSend {
        id_hint: Option<SendId>,
        source: SendSource,
        dest: BusId,
        level_db: f32,
        pre_fader: bool,
        enabled: bool,
    },
    /// Remove an aux send. Emits `AudioEvent::AuxSendRemoved` when a send
    /// with this id existed; otherwise a no-op.
    RemoveAuxSend {
        send_id: SendId,
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

    // -- Audition preview (doc #175) --
    /// Preview an arbitrary audio file through the engine, starting at
    /// `start_frame` (clamped to the file length). The file may be an imported
    /// pool asset or an un-imported file straight off the filesystem; any
    /// format the workspace decoder accepts works. The engine decodes it off
    /// the audio thread and previews it independently of the arrangement,
    /// transport, and undo — it is never an `AudioClip` and does not move the
    /// main playhead. Uses the loop / sync-to-tempo options last set via
    /// [`AudioCommand::SetAuditionOptions`]. A decode failure surfaces as
    /// `AudioEvent::Error`. Replaces any preview already playing.
    AuditionFile {
        path: PathBuf,
        start_frame: u64,
    },
    /// Stop the current audition preview. Emits `AudioEvent::AuditionStopped`
    /// when a preview was actually playing; stopping an idle audition is a
    /// silent no-op.
    StopAudition,
    /// Set the audition preview options. `loop_enabled` wraps the preview at
    /// its end instead of stopping; `sync_to_tempo` time-stretches (varispeed)
    /// the preview so its loop length snaps to the project tempo. The options
    /// persist across `AuditionFile` commands and take effect immediately on
    /// any preview currently playing.
    SetAuditionOptions {
        loop_enabled: bool,
        sync_to_tempo: bool,
    },

    // -- MIDI Learn & hardware controller mapping (doc #167 §2 E2) --
    /// Insert or replace (by `binding.id`) one hardware-control → target
    /// mapping in the engine's active binding set. Sent when the app learns
    /// a control or edits a binding's range / mode / takeover. The engine
    /// echoes the resolved binding back via `AudioEvent::MidiBindingChanged`
    /// so app state stays a pure projection of engine events (no read-getters,
    /// doc #105).
    SetMidiBinding {
        binding: MidiBinding,
    },
    /// Remove the active binding with this id. Emits
    /// `AudioEvent::MidiBindingCleared` on success (and is a silent no-op if
    /// no such binding is active).
    ClearMidiBinding {
        id: BindingId,
    },
    /// Replace the entire active binding set with `map`'s bindings. Used by
    /// controller-preset load and by project-load replay; the engine emits a
    /// `MidiBindingChanged` per resulting binding so the app can rebuild its
    /// `MidiMapState` from events alone.
    SetControllerMap {
        map: ControllerMap,
    },
    /// Drop every active binding (e.g. switching to an empty preset).
    ClearAllMidiBindings,
    /// Pick (`Some`) or clear (`None`) the dedicated control-surface MIDI
    /// input port the engine listens to for CC / note control messages,
    /// independent of the per-track MIDI inputs.
    SetControlSurfaceInput {
        device: Option<String>,
    },
    /// Arm MIDI Learn for `target`: the next qualifying control-surface
    /// message is captured and reported via `AudioEvent::MidiLearnCaptured`
    /// instead of being applied, then learn mode exits automatically.
    EnterMidiLearn {
        target: MidiTarget,
    },
    /// Cancel an armed MIDI Learn without capturing anything (Esc / re-click).
    CancelMidiLearn,

    // -- Freeze commands --
    /// Kick off an offline render of the track's post-instrument/post-FX
    /// output to `cache_path`. The render produces a freeze-cache WAV
    /// containing the full track output (including SVS-rendered vocals).
    /// Emits progress events, then `FreezeCompleted` on success or
    /// `FreezeError`/`FreezeCancelled` on failure/cancel.
    FreezeTrack {
        track_id: TrackId,
        cache_path: String,
    },
    /// Attach or detach a decoded freeze cache buffer to/from a track.
    /// Used on project load to rehydrate frozen tracks without re-rendering,
    /// and on unfreeze to clear the frozen source.
    SetTrackFrozenSource {
        track_id: TrackId,
        /// `Some(source)` attaches the frozen buffer for playback.
        /// `None` detaches it, restoring live synth+FX playback.
        source: Option<FrozenSource>,
    },
    /// Detach the frozen source from a track and resume live synth+FX playback.
    UnfreezeTrack {
        track_id: TrackId,
    },
    /// Cancel an in-flight freeze render. Cooperative: the render polls
    /// the shared cancel flag between chunks and aborts cleanly.
    CancelFreeze,

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
    /// **Engine-internal** — not sent by the GUI. Posted by the reference
    /// analysis worker (via the engine's retry-command channel) once a
    /// freshly-loaded reference has been decoded and loudness-measured,
    /// carrying the decoded stereo-interleaved PCM and integrated LUFS so
    /// the engine can store them into the registered reference entry.
    ReferenceAnalyzed {
        id: ReferenceId,
        pcm: Arc<Vec<f32>>,
        integrated_lufs: f32,
    },

    /// Break the engine-thread loop and let the thread exit cleanly.
    /// Required because the engine thread holds its own `Sender` clone
    /// (`cmd_tx_retry`) for the retry path, which prevents the channel
    /// from ever returning `Disconnected` even after every external
    /// sender has dropped. Sent by `AudioEngine::shutdown` / `Drop`.
    ShutDown,
}
