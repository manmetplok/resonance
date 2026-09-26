//! GUI → engine command enum.
use std::path::PathBuf;
use std::sync::Arc;

use resonance_common::{
    BindingId, CompSegment, ControllerMap, MidiBinding, MidiTarget, TakeGroup, TakeGroupId, TakeId,
};

use resonance_common::{AutomationLane, AutomationTarget, DeviceParam, PlaybackSource};

use super::{
    ABSource, BusId, ClipId, ExportSettings, FadeCurve, FrozenSource, MeasureSource, MidiNote,
    PluginInstanceId, ReferenceId, SamplePos, SendId, SendSource, SignaturePoint, StemBitDepth,
    StemSource, StemTarget, TempoPoint, TrackId, TrackOutput, WarpAlgorithm, WarpMarker,
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
    /// Route `source`'s audio into `plugin`'s external sidechain (key)
    /// input, replacing any route that instance already had. The plugin
    /// must declare a key port (the compressor and the gate do); a route
    /// onto one that doesn't is stored but inert, so a preset swap that
    /// changes the plugin can't silently misroute audio.
    ///
    /// The key is delivered one block late by construction — see
    /// [`crate::types::sidechain`] for why that is the right trade.
    SetSidechainRoute {
        plugin: PluginInstanceId,
        source: SendSource,
        enabled: bool,
    },
    /// Remove `plugin`'s key route, so it falls back to keying off its
    /// own input.
    ClearSidechainRoute {
        plugin: PluginInstanceId,
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
    /// Raise the engine's media-pool id allocator above `above`, so a
    /// later `ImportAudioToPool` cannot hand out an id a loaded project
    /// already uses.
    ///
    /// The allocator is engine-thread-local and starts at 1 each
    /// session; a project's assets keep the ids they were saved with,
    /// and the engine is never told about them (the app restores the
    /// pool itself). Without this the first import after opening a
    /// project took id 1 — which some existing asset already had — and
    /// every clip referencing that id silently started playing the new
    /// file (ba doc #276 BUG 2). Clip ids have always had this
    /// high-water treatment via `LoadClipFromWav`; assets did not.
    ReserveAssetIds {
        above: crate::types::AssetId,
    },
    /// Cut `clip_id` in two at `at_sample` (an absolute timeline
    /// position). The original keeps the head and its id; the tail
    /// becomes a second clip with `new_clip_id`, playing the same source
    /// from the split point.
    ///
    /// Both halves are non-destructive trims of the same source, so no
    /// audio is copied for a memory-mapped clip. A split outside the
    /// clip's audible span is a no-op — there would be nothing on one
    /// side of it.
    SplitClip {
        clip_id: ClipId,
        new_clip_id: ClipId,
        at_sample: SamplePos,
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
    /// Add an audio track. Since ARCH-04 D-4 the app allocates every
    /// track id (`Resonance::allocate_track_id`) and `id` is mandatory;
    /// the engine has no allocator of its own left and refuses a
    /// collision (`EngineErrorKind::Internal`,
    /// `tracks::reject_if_track_id_in_use`) rather than replacing the
    /// live track.
    AddTrack {
        id: TrackId,
        name: Option<String>,
    },
    RemoveTrack {
        track_id: TrackId,
    },
    /// Register an app-side sub-track with the audio engine so the mixer
    /// can drive it from its parent plugin's output port. The app
    /// allocates the id itself — the same `Resonance::allocate_track_id`
    /// every other track add draws from (ARCH-04 D-4; tracks and
    /// sub-tracks have shared one id space since before that) — and
    /// passes it here. Called after `AudioEvent::PluginAdded` for every
    /// non-main output port on a multi-output plugin.
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
    /// Switch an external-instrument track between playing its recorded
    /// takes and re-driving the hardware live (doc #257). Engine-owned
    /// state like monitor/record-arm; echoed back via
    /// `AudioEvent::TrackPlaybackSourceChanged`.
    SetTrackPlaybackSource {
        track_id: TrackId,
        source: PlaybackSource,
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
        /// The app allocates every plugin instance id
        /// (`Resonance::allocate_plugin_id`, ARCH-04 D-1); the engine
        /// only ever honours it. Rejected with `EngineError::internal`
        /// (`AudioEvent::Error`) if `id` is already live, rather than
        /// silently re-numbering the add.
        id: PluginInstanceId,
    },
    RemovePlugin {
        track_id: TrackId,
        instance_id: PluginInstanceId,
    },
    /// Reorder a track's insert chain: move `instance_id` to `to_index`,
    /// shifting the plugins between its old and new slot by one. Order is
    /// audible — an EQ before a compressor is a different sound from an EQ
    /// after it — and before this command the only way to reorder a chain
    /// was to tear it down and rebuild it.
    ///
    /// `to_index` is clamped to the last slot, so an out-of-range value
    /// moves the plugin to the end rather than failing. Moving a plugin to
    /// the slot it already occupies is a no-op that still confirms with
    /// [`AudioEvent::PluginMoved`](super::AudioEvent::PluginMoved).
    ///
    /// **Slot 0 is structural on instrument tracks**: `plugins().first()` is
    /// the instrument — it is what receives MIDI, what keeps running while
    /// the track's FX are bypassed, and what every sub-track inherits its
    /// latency from (`latency::chain_latencies`). The engine performs
    /// whatever move it is asked to, so a caller that only means to reorder
    /// *effects* must keep the instrument pinned at index 0 itself.
    ///
    /// An unknown track, or an `instance_id` that is not on that track's
    /// chain, leaves the chain untouched and reports `AudioEvent::Error`.
    MovePlugin {
        track_id: TrackId,
        instance_id: PluginInstanceId,
        to_index: usize,
    },
    ScanPlugins,
    /// Look for newly installed plugins WITHOUT disturbing anything that
    /// is already running (ba todo #1307, finding X10).
    ///
    /// [`ScanPlugins`](Self::ScanPlugins) drops every instantiated plugin
    /// and reloads every bundle, which is only safe before anything has
    /// been instantiated — so before this existed, installing a plugin
    /// meant restarting the app. This one is purely additive: loaded
    /// bundles stay loaded at the address their live instances came from,
    /// and only files not yet loaded are opened. Answers with
    /// `AudioEvent::PluginsScanned` (the whole catalog) plus
    /// `AudioEvent::PluginScanFailed` when a bundle refused to load.
    RescanPlugins,
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
    /// Replace the comp — the ordered, non-overlapping cover of the loop
    /// slot — of take group `group_id`. The mix graph plays the comped
    /// cover, switching the source take clip per [`CompSegment`] with a
    /// short equal-power crossfade at each seam, on both realtime playback
    /// and offline bounce.
    ///
    /// Setting a comp does not clear an active take: an active take still
    /// overrides the comp until cleared with
    /// [`AudioCommand::SetActiveTake`]`(_, None)`. Confirmed by
    /// `AudioEvent::TakeCompChanged`; an unknown group is ignored.
    SetTakeComp {
        group_id: TakeGroupId,
        segments: Vec<CompSegment>,
    },
    /// Solo one whole take of group `group_id` for full-slot playback,
    /// overriding the comp — or clear the override with `None` so the comp
    /// plays again. Confirmed by `AudioEvent::ActiveTakeChanged`; an unknown
    /// group, or a take id the group does not hold, is ignored.
    SetActiveTake {
        group_id: TakeGroupId,
        take_id: Option<TakeId>,
    },
    /// Remove take `take_id` from take group `group_id`: drop the take,
    /// re-cover the slot from the takes that remain, park the take's
    /// recording so it stops sounding, and republish the comp table.
    ///
    /// Confirmed by `AudioEvent::TakeRemoved`, followed by
    /// `TakeCompChanged` / `ActiveTakeChanged` when the removal moved
    /// either. An unknown group, or a take the group does not hold, is
    /// ignored (the handlers' standing missing-lookup convention).
    ///
    /// **The comp re-covers; it is never left dangling.** The removal
    /// itself is `TakeGroup::remove_take` in `resonance-common` — one
    /// definition shared with anything mirroring the group, exactly as
    /// `effective_cover` is one definition of what a group sounds like
    /// (ba todo #1395). Segments naming the removed take are dropped and
    /// the holes handed to the take the cover now falls back to, so the
    /// only stretch of the slot that changes is the one the removed take
    /// used to play.
    ///
    /// **Removing the last take removes the group** (ba todo #1397). Three
    /// reasons, and the first is a trap: with no takes left the comp is
    /// empty, and an empty comp is exactly the state in which the cover
    /// falls back to *the most recent pass* — the take just deleted. A
    /// group with no takes resolves to no spans today, but leaving the
    /// hazard guarded by one early return in `effective_cover` is how it
    /// comes back. Second, a group *is* its takes (doc #165): an empty
    /// lane is chrome the user cannot record into or comp. Third, an
    /// empty group keeps its slot — which never moves once bound (ba todo
    /// #1392) — so the next cycle-record run over that region would join
    /// the lane the user had just deleted instead of starting a new one.
    /// The echo in that case is `TakeGroupRemoved`, not `TakeRemoved`.
    ///
    /// **The WAV survives — by decision, not by accident.** The handler
    /// holds `HandlerState`, so `project_dir` is right there and unlinking
    /// `audio/clip_N.wav` would be two lines. It must not, because
    /// deleting a user's recording is irreversible and the removal is not:
    /// an undo restores the take through
    /// [`AudioCommand::RestoreTakeGroups`] carrying the same `clip_ref`,
    /// so a deleted file would make undo silently lossy. The clip is
    /// *parked* instead — moved out of the shared clip list, which is what
    /// stops it playing, and kept in memory so the undo is instant. The id
    /// cannot be re-issued while the file lives, because `next_clip_id`
    /// only ever rises within a session and is reserved past every
    /// restored `clip_ref` (ba todo #1393). Reclaiming orphaned audio is a
    /// project-level operation the user asks for, not something a command
    /// on the audio path does behind their back.
    RemoveTake {
        group_id: TakeGroupId,
        take_id: TakeId,
    },
    /// Remove take group `group_id` outright: the lane, every take in it,
    /// and its comp. Confirmed by `AudioEvent::TakeGroupRemoved`; an
    /// unknown group is ignored.
    ///
    /// Every audio take in the group is parked exactly as
    /// [`AudioCommand::RemoveTake`] parks one, so no recording is deleted
    /// and an undo restores the whole lane.
    ///
    /// Parking is what makes the removal *inaudible*, and it is the whole
    /// reason this is a command rather than a comp edit. A take's
    /// recording is an ordinary [`AudioClip`](crate::types::AudioClip) in
    /// the shared clip list; it stays silent only because the comp table
    /// marks it *governed* and the clip phase skips it. Drop the group
    /// without parking and every take in it stops being governed — so
    /// deleting a lane would play all of its passes at once, on top of
    /// each other, at full gain.
    RemoveTakeGroup {
        group_id: TakeGroupId,
    },
    /// Replace the engine's take-group store **wholesale** with `groups`,
    /// republish the comp playback table, and raise both the take-group id
    /// allocator and the clip id allocator above every id the restored
    /// groups already hold.
    ///
    /// This is how a saved project's take lanes get back into the engine
    /// (ba todo #1394). Take groups are born in the engine — cycle
    /// recording calls `capture_take_event` as each pass rolls — but a project
    /// load has no capture to be born from, and until this command
    /// existed nothing else wrote the store: the app restored its own
    /// mirror, drew the lanes, and the engine rendered **silence** for a
    /// comp the user could see. Doc #165 requires a comp to persist
    /// across save/load, on playback *and* on bounce; both read the same
    /// published table, so republishing here is what makes a loaded comp
    /// audible without touching the transport.
    ///
    /// **Wholesale, not additive.** The two senders — a disk load and an
    /// undo/redo diff replay — each rebuild the app-side mirror from
    /// scratch and send the result, so replacing keeps engine and app
    /// mirror identical by construction. Merging instead would resurrect
    /// takes an undo had just deleted.
    ///
    /// **Wholesale covers the recordings too** (ba todo #1397). The store
    /// is not the only thing a removal touched: [`AudioCommand::RemoveTake`]
    /// parks the take's clip out of the shared clip list, so restoring
    /// must un-park every recording the incoming groups claim — otherwise
    /// undoing a deletion brings the take back on screen and leaves it
    /// silent. It must also park every take clip the incoming groups
    /// *stop* claiming, which is what a redo of a deletion looks like on
    /// this path: without it the clip would be left registered and
    /// un-governed, and the redone deletion would play the take at full
    /// gain on the ordinary clip path.
    ///
    /// **Restored groups are reused, not shadowed.** Since todo #1392 a
    /// cycle-record run resolves its group by looking up the store for a
    /// lane on the same track over the same loop region, so a pass
    /// recorded after a load joins the *restored* lane rather than
    /// starting a second one beside it — one lane per slot, across a save
    /// as well as across a stop.
    ///
    /// **The allocators — two of them.** `next_take_group_id` is
    /// engine-thread-local and starts at 1 each session;
    /// `loop_record_group_for` is its only consumer, and only for a run
    /// that matches no existing lane. Without the high-water bump the
    /// first cycle-record run after a load re-issued group id 1, and since
    /// `push_take` allocates take ids *from the group*, that run's first
    /// take took id 0 — silently replacing a restored take in the app's
    /// `(group_id, take_id)`-keyed mirror. `next_clip_id` gets the same
    /// treatment here (ba todo #1393): an audio take's `clip_ref` owns
    /// `audio/clip_N.wav`, and take clips never travel the
    /// [`AudioCommand::LoadClipFromWav`] path that would otherwise reserve
    /// it, so without the bump the next recording or import **overwrote a
    /// restored take's WAV**. Pool assets get it from
    /// [`AudioCommand::ReserveAssetIds`]. Take ids inside a restored
    /// group need no reservation of their own: `push_take` derives them
    /// from the group's own contents, so they are correct the moment the
    /// group is present.
    ///
    /// Deliberately silent — no echo. The sender is restoring state it
    /// already holds, so an echo would only invite it to re-apply its own
    /// input, exactly as [`AudioCommand::ReserveAssetIds`] does.
    RestoreTakeGroups {
        groups: Vec<TakeGroup>,
    },
    /// Put one restored **take** clip's recorded WAV back into the
    /// engine's clip list on project load (ba todo #1402).
    ///
    /// [`AudioCommand::RestoreTakeGroups`] rebuilds the *groups*; this
    /// rebuilds the audio they name. Both are needed and neither implies
    /// the other: a take clip enters the engine only through the capture
    /// path (`roll_audio_pass` pushes the `AudioClip` straight into the
    /// list and emits no clip command at all), so a reloaded project had
    /// the comp table resolving to clip ids the engine did not hold, and
    /// the comp rendered **silence on playback and on bounce** — the very
    /// failure `RestoreTakeGroups` was added to fix, surviving in its
    /// other half.
    ///
    /// **Why not [`AudioCommand::LoadClipFromWav`].** Same mmap, same
    /// worker, one decisive difference: that command echoes
    /// `AudioEvent::ClipImported`, and the app's handler pushes a
    /// `ClipState` for any clip it does not already know. A take clip is
    /// not a timeline clip (ba todo #1396) — it must not appear in
    /// `Resonance::clips`, or it would be drawn on the timeline, saved
    /// into the project's clip list, and then also play raw on the next
    /// load. So this command is **silent**, for the same reason
    /// `RestoreTakeGroups` is: the app is the sender and the mirror, and a
    /// freshly loaded project must not come up dirty.
    ///
    /// **Ordering.** Sent *after* `RestoreTakeGroups`, so a take clip is
    /// governed by the published comp table from the first instant it
    /// exists. The reverse order leaves a window in which the raw,
    /// overlapping passes are ungoverned and all play at once on the
    /// ordinary clip path. The window this order does leave — a table
    /// naming a clip not yet loaded — is benign: `mix_track_comp` skips a
    /// span whose clip it cannot find, and the load is asynchronous
    /// anyway, so no send order could close it.
    ///
    /// **Idempotent.** A clip id already present in the list is left
    /// alone. The sender also fires on the undo/redo diff replay, where
    /// the clips are already loaded, and (once take removal lands) a
    /// restore may have just un-parked this very clip.
    ///
    /// `start_sample` is the take's `extent.start`: `RolledAudioTake::extent`
    /// is defined as the rolled clip's own `[start_sample, +duration)`, so
    /// the persisted extent is an exact record of where capture placed the
    /// clip. It matters — `mix_track_comp` intersects every span with
    /// `clip.start_sample .. + duration_frames()`, so a clip restored to
    /// the wrong origin reads the wrong audio or none.
    ///
    /// Trims and fades are absent because a take clip has none: capture
    /// writes it whole, and comping trims by choosing spans, not by
    /// editing the clip.
    LoadTakeClipFromWav {
        clip_id: ClipId,
        track_id: TrackId,
        start_sample: SamplePos,
        path: PathBuf,
        name: String,
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
    /// [`AudioCommand::AddTrack`] with `id = target_track_id`
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
    /// Offline "measure the mix": render one or more mix slices (a track,
    /// a bus, or the whole master) over ONE shared range and feed them to
    /// the BS.1770 meters, WITHOUT writing anything to disk and without
    /// touching project or transport state (ba todo #1218, doc #273).
    ///
    /// Every target is rendered over the same range, so the results are
    /// directly comparable — which is what lets the control layer's
    /// `meter.stems` be a pure enumeration of sources over this one
    /// command rather than a second engine pass. A track target includes
    /// its sub-tracks (`stem_filter`), so a multi-output instrument is
    /// measured whole and cannot suffer the drum-bleed attribution error
    /// that mute-and-bounce measurement does.
    ///
    /// Rendering runs on a worker thread (like
    /// [`AudioCommand::ExportStems`]) and is refused while another
    /// offline render — bounce, export, freeze or stem export — holds the
    /// renderer, or while the transport is rolling. Emits exactly ONE
    /// terminal event: `MixMeasured` with one measurement per target, or
    /// `MixMeasureError` if the whole command failed. Both echo
    /// `measure_id`, so a caller with several measurements outstanding
    /// can tell which one answered.
    MeasureMix {
        /// Opaque correlation token, echoed unchanged on whichever
        /// terminal event this command produces (ba todo #1243).
        ///
        /// The engine never interprets it — it is the caller's handle on
        /// its own request. `resonance-app` passes the control job's id,
        /// which is what lets `meter.*` match a result to the request
        /// that asked for it instead of to "the newest pending measure
        /// job", a correlation that only held while exactly one
        /// measurement could ever be in flight.
        measure_id: u64,
        /// The mix slices to measure. No paths: nothing is written.
        targets: Vec<StemSource>,
        /// Shared measurement window in engine samples. `None` measures
        /// the full project range (every audio + MIDI clip), matching the
        /// project bounce. Ignored by [`MeasureSource::Live`].
        range: Option<(SamplePos, SamplePos)>,
        /// Render offline, or read the live master meter.
        source: MeasureSource,
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
    /// Write `{project_dir}/audio/clip_{id}.wav` for every in-engine
    /// audio clip that has none yet, silently (code review FU-V5b). Sent
    /// whenever the app captures an undo snapshot, so a full-reload undo
    /// finds the audio of every clip the snapshot names.
    PersistClipWavs,
    /// Batch save all plugin states for project save.
    SaveAllPluginStates,
    /// Remove all tracks, clips, and plugins (for project load).
    ClearAll,

    // -- Instrument track commands --
    /// Add an instrument track. See [`AudioCommand::AddTrack`] for how
    /// `id`/`name` are honoured.
    AddInstrumentTrack {
        id: TrackId,
        name: Option<String>,
    },

    // -- Vocal track commands --
    /// Add a vocal track. Engine-side this is an instrument-shaped track
    /// (accepts live MIDI) but its playback path runs through the audio
    /// clip pipeline so the SVS-rendered WAV is what's heard. See
    /// [`AudioCommand::AddTrack`] for how `id`/`name` are honoured.
    AddVocalTrack {
        id: TrackId,
        name: Option<String>,
    },

    // -- MIDI clip commands --
    /// `clip_id` is mandatory (D-7c): the app allocates it (today's general
    /// clip allocator, `ComposeState::fresh_derived_clip_id`) and carries it
    /// here rather than letting the engine invent one. The engine refuses a
    /// collision with `EngineErrorKind::Internal` instead of overwriting the
    /// live clip — see `reject_if_clip_id_in_use`.
    CreateMidiClip {
        clip_id: ClipId,
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

    /// Replace `clip_id`'s entire note array in one atomic edit; emits a
    /// single `AudioEvent::MidiNotesEdited` carrying the result. The
    /// bulk write behind the control endpoint's `notes.insert_many` /
    /// `notes.replace_all` (ba doc #269 FR-5), where issuing N single
    /// `AddMidiNote` commands would cost N undo entries and N echoes.
    /// The caller passes the final array; the engine does no merging.
    SetMidiClipNotes {
        clip_id: ClipId,
        notes: Vec<MidiNote>,
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

    /// Hand the engine the automatable device parameters of the device
    /// preset selected on an external-instrument track (architecture doc
    /// #201 §4, epic #40). The engine stores each [`DeviceParam`] on the
    /// engine-side track keyed by `DeviceParam::id` so a
    /// `AutomationTarget::DeviceParam` lane can be mapped to its bound
    /// CC/NRPN at render time **without reaching back across the
    /// command/event boundary** (no engine getters). The app sends this
    /// when a device preset is selected or changed on the track.
    ///
    /// An empty `params` vec clears the map (acts as a "no device
    /// selected" / clear command). The engine replaces the whole map on
    /// every command — it is not a merge — and confirms with
    /// [`crate::types::AudioEvent::TrackDeviceParamsApplied`]. Per-block
    /// lane evaluation that actually emits the MIDI is a later todo (E3);
    /// this command only plumbs the binding map into the engine.
    SetTrackDeviceParams {
        track_id: TrackId,
        params: Vec<DeviceParam>,
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
    /// Add a bus. The app allocates every bus id (ARCH-04 D-3, same shape
    /// as [`AudioCommand::AddPlugin`]'s `id`); the engine only ever
    /// honours it, refusing with `EngineError::internal` if `id` is
    /// already live rather than replacing the bus it names.
    AddBus {
        id: BusId,
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
        /// The app allocates every plugin instance id
        /// (`Resonance::allocate_plugin_id`, ARCH-04 D-1); the engine
        /// only ever honours it. Rejected with `EngineError::internal`
        /// (`AudioEvent::Error`) if `id` is already live, rather than
        /// silently re-numbering the add.
        id: PluginInstanceId,
    },
    RemovePluginFromBus {
        bus_id: BusId,
        instance_id: PluginInstanceId,
    },
    /// Reorder a bus's insert chain: move `instance_id` to `to_index`,
    /// shifting the plugins between its old and new slot by one. The bus
    /// twin of [`MovePlugin`](Self::MovePlugin), and audible for the
    /// same reason — a bus chain is applied front to back over the
    /// SUMMED group, so an EQ before a compressor is a different sound
    /// from an EQ after it.
    ///
    /// `to_index` is clamped to the last slot, so an out-of-range value
    /// moves the plugin to the end rather than failing. Moving a plugin
    /// to the slot it already occupies is a no-op that still confirms
    /// with [`AudioEvent::BusPluginMoved`](super::AudioEvent::BusPluginMoved).
    ///
    /// Unlike a track chain, a bus chain has no structural slot 0: every
    /// entry is an effect over the group sum, so any order is valid.
    ///
    /// An unknown bus, or an `instance_id` that is not on that bus's
    /// chain, leaves the chain untouched and reports `AudioEvent::Error`.
    MovePluginInBus {
        bus_id: BusId,
        instance_id: PluginInstanceId,
        to_index: usize,
    },

    // -- Aux sends + return busses --
    /// Mark a bus as an aux *return* bus (or clear the flag). Emits
    /// `AudioEvent::BusRoleChanged`. No-op if the bus does not exist.
    SetBusRole {
        bus_id: BusId,
        is_return: bool,
    },
    /// Create a new aux send from a track or bus into a return bus, under
    /// an app-allocated `id` (ARCH-04 D-2, same shape as
    /// [`AudioCommand::AddPlugin`]'s `id`). The engine runs cyclic-route
    /// validation before registering — a send routing a bus to itself, or
    /// to a destination whose own sends already reach the source bus, is
    /// rejected with `AudioEvent::AuxSendRejected` and not stored — and
    /// refuses the add outright with `EngineError::internal` if `id` is
    /// already live, rather than silently turning the create into an edit
    /// of the send that id names. On success the engine emits
    /// `AudioEvent::AuxSendChanged` with the resolved send.
    AddAuxSend {
        id: SendId,
        source: SendSource,
        dest: BusId,
        level_db: f32,
        pre_fader: bool,
        enabled: bool,
    },
    /// Edit an existing aux send in place — re-route / level / pre-post /
    /// enable, covered in one command since every edit resends the send's
    /// full state under its own `id`. The same cyclic-route validation as
    /// [`AudioCommand::AddAuxSend`] runs, excluding the send's own current
    /// edge. A no-op if `id` does not name a live send (an edit racing its
    /// own removal), rather than an error: unlike a duplicate `id` on
    /// [`AudioCommand::AddAuxSend`], there is no caller-invariant violation
    /// to report here. On success the engine emits
    /// `AudioEvent::AuxSendChanged` with the resolved send.
    SetAuxSend {
        id: SendId,
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
        /// The app allocates every plugin instance id
        /// (`Resonance::allocate_plugin_id`, ARCH-04 D-1); the engine
        /// only ever honours it. Rejected with `EngineError::internal`
        /// (`AudioEvent::Error`) if `id` is already live, rather than
        /// silently re-numbering the add.
        id: PluginInstanceId,
    },
    RemovePluginFromMaster {
        instance_id: PluginInstanceId,
    },
    /// Reorder the master insert chain: move `instance_id` to
    /// `to_index`, shifting the plugins between its old and new slot by
    /// one. The master twin of
    /// [`MovePluginInBus`](Self::MovePluginInBus), and audible for the
    /// same reason — the chain runs front to back over the finished mix.
    /// On the master it also decides whether the chain works at all: a
    /// limiter holding a ceiling must be last, because anything after it
    /// can push the sum back over that ceiling.
    ///
    /// `to_index` is clamped to the last slot, so an out-of-range value
    /// moves the plugin to the end rather than failing. Moving a plugin
    /// to the slot it already occupies is a no-op that still confirms
    /// with [`AudioEvent::MasterPluginMoved`](super::AudioEvent::MasterPluginMoved).
    ///
    /// Like a bus chain — and unlike a track's — the master has no
    /// structural slot 0: every entry is an effect over the summed mix,
    /// so any order is valid.
    ///
    /// An `instance_id` that is not on the master chain leaves it
    /// untouched and reports `AudioEvent::Error`.
    MovePluginInMaster {
        instance_id: PluginInstanceId,
        to_index: usize,
    },
    /// Bypass every effect plugin on a track. Instrument plugins
    /// (slot 0 on instrument tracks) keep running.
    ///
    /// The change is not instantaneous: the mixer crossfades the chain
    /// out (or back in) over a few milliseconds, so bypassing a reverb
    /// mid-playback fades its tail instead of truncating it.
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
    /// Bypass **one** slot of a chain, wherever it lives — a track, a
    /// sub-track, a bus or the master (ba doc #275 finding X3). The slot
    /// is named by its plugin instance id, which is unique across every
    /// chain, so no chain-kind discriminant is needed.
    ///
    /// Like the whole-chain bypasses, the transition is crossfaded rather
    /// than switched. Two behaviours follow from what the plugin declares:
    ///
    /// - it declares a `CLAP_PARAM_IS_BYPASS` parameter → the host drives
    ///   that parameter and keeps calling the plugin, so its latency (and
    ///   with it the whole compensation table) is unaffected;
    /// - it does not → the mixer stops calling it once the fade lands, and
    ///   its latency leaves the chain, exactly as a whole bypassed chain's
    ///   already does.
    ///
    /// Confirmed with [`AudioEvent::PluginBypassChanged`](super::AudioEvent::PluginBypassChanged).
    /// An unknown `instance_id` is a no-op and reports `AudioEvent::Error`.
    SetPluginBypass {
        instance_id: PluginInstanceId,
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
    /// `ReferenceAnalysisProgress`) or `ReferenceLoadFailed`. `id` is
    /// mandatory (ARCH-04 D-5): the app is the only allocator left for
    /// reference ids, and the engine refuses a collision with
    /// `EngineErrorKind::Internal` rather than silently replacing the
    /// live entry.
    LoadReferenceTrack {
        id: ReferenceId,
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
    /// Deselect the active reference, so the A/B monitor has nothing to
    /// audition (it falls through to the mix). Sent by an undo that
    /// restores "nothing selected"; the user can only change the
    /// selection, never clear it. Silent: the app has already applied it.
    ClearActiveReference,
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
    /// Add a comparison marker to a reference at a sample position,
    /// under a marker id the app allocated (FU-A5a: the app restores saved
    /// markers the engine never hears about, so only it can pick an id
    /// none of them uses). Re-adding an id the reference already holds
    /// moves that marker. Emits `AudioEvent::RefMarkerAdded`.
    AddRefMarker {
        ref_id: ReferenceId,
        marker_id: u32,
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
    /// Ask the engine for the current I/O latency figures (graph-
    /// reported capture / playback delay of the native PipeWire streams,
    /// in samples at the engine rate) and reply with
    /// `AudioEvent::IoLatencyReport` (doc #260 finding #13). Capture is
    /// 0 while no input stream is open; both are 0 on the cpal fallback,
    /// which cannot report latency.
    QueryIoLatency,
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
