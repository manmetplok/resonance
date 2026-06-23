//! Engine control thread: owns the command/event loop and the
//! per-command handler dispatch. All mutable engine state that must
//! outlive a single command lives in [`HandlerState`]; the shared
//! references to `Arc<RwLock<...>>` project state, the event sender, and
//! the retry-command sender live in [`HandlerCtx`].
//!
//! Handlers are free functions in the submodules (`transport`, `tracks`,
//! `clips`, `midi`, `plugins`, `busses`). They take `&HandlerCtx` +
//! `&mut HandlerState` + the command payload and execute synchronously.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use crate::clap_host::{ClapBundle, SyncClapInstance};
use crate::midi_clock::{
    ClockTempoTracker, MidiClockEvent, MidiClockReceiver, MidiClockSender,
};
use crate::midi_hardware::{LiveControlEvent, LiveMidiEvent};
use crate::mixer::MidiStash;
use crate::recording::RecordingState;
use crate::types::*;
use resonance_common::{TakeGroupId, TimelineRange};

use super::midi::MidiHardwareState;
use super::{
    audition, bounce, bounce_realtime, busses, clips, master, midi, midi_map, plugins, reference,
    scan, tracks, transport, vocal_analysis, SharedState,
};

/// Read-only handle to shared project state and channels. Passed by
/// reference into every handler so they can lock the relevant maps and
/// emit events without taking ownership.
pub(crate) struct HandlerCtx<'a> {
    pub shared: &'a Arc<SharedState>,
    pub tracks: &'a Arc<RwLock<IndexMap<TrackId, Track>>>,
    pub busses: &'a Arc<RwLock<IndexMap<BusId, Bus>>>,
    pub master: &'a Arc<RwLock<MasterBus>>,
    pub clips: &'a Arc<RwLock<Vec<AudioClip>>>,
    pub midi_clips: &'a Arc<RwLock<Vec<MidiClip>>>,
    pub plugins: &'a Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    pub tempo_map: &'a Arc<arc_swap::ArcSwap<TempoMap>>,
    pub latency_comp: &'a Arc<arc_swap::ArcSwap<crate::latency::LatencyComp>>,
    pub monitor_prod: &'a Arc<Mutex<ringbuf::HeapProd<f32>>>,
    pub event_tx: &'a Sender<AudioEvent>,
    pub cmd_tx_retry: &'a Sender<AudioCommand>,
    pub sample_rate: u32,
    pub buf_frames: usize,
    pub quantum: usize,
}

/// Per-track state for ongoing live MIDI recording. Kept on the
/// engine control thread so it never leaks into the audio callback.
pub(crate) struct RecordingMidiState {
    /// MIDI clip currently being recorded into.
    pub clip_id: ClipId,
    /// Absolute tick at the clip's start sample. Used to convert the
    /// per-event playhead tick into the clip-relative `start_tick`.
    pub clip_start_tick: u64,
    /// Currently held notes: pitch → index in the clip's `notes` vec.
    /// On NoteOff we look the note up here, set its `duration_ticks`,
    /// and remove the entry. Stuck NoteOns get closed at transport stop.
    pub open_notes: HashMap<u8, usize>,
}

/// Bookkeeping for an in-flight cycle-record (loop-record) run. Created
/// in [`transport::begin_recording_stream`] when loop-record mode and a
/// loop range are both active, advanced at each loop seam, and torn down
/// when recording stops. Lives on the engine control thread so it never
/// touches the audio callback.
pub(crate) struct LoopRecordSession {
    /// The loop region being cycled over, in sample frames. Reported on
    /// every `AudioEvent::TakeCaptured` as the take's slot.
    pub slot: TimelineRange,
    /// Zero-based index of the pass currently being captured. Bumped at
    /// each seam after the completed pass's takes are emitted.
    pub pass_index: u32,
    /// Stable take-group id per track for this run, allocated lazily the
    /// first time a track produces a take. Keeps all passes of a track
    /// folded into a single group on the app side.
    pub groups: HashMap<TrackId, TakeGroupId>,
}

/// Mutable engine-thread-local state that persists across command
/// dispatches: monotonic id counters, the recording session, the loaded
/// CLAP bundles, and the concurrent-import counter.
pub(crate) struct HandlerState {
    pub next_track_id: TrackId,
    pub next_bus_id: BusId,
    pub next_clip_id: ClipId,
    /// Monotonic id allocator for media-pool assets imported via
    /// `AudioCommand::ImportAudioToPool`. Independent of `next_clip_id`:
    /// asset WAVs are named `asset_{id}.wav`, clip WAVs `clip_{id}.wav`,
    /// so the two counters never collide on disk.
    pub next_asset_id: AssetId,
    pub next_plugin_id: PluginInstanceId,
    pub next_send_id: SendId,
    /// Aux sends keyed by id, in insertion order. Engine-thread-local
    /// (never read from the audio callback), so plain data — see
    /// [`AuxSend`]. The source of truth for cyclic-route validation.
    pub aux_sends: IndexMap<SendId, AuxSend>,
    /// Monotonic id allocator for cycle-record take groups. One group is
    /// handed out per armed track per loop-record run (see
    /// [`LoopRecordSession::groups`]).
    pub next_take_group_id: TakeGroupId,
    pub rec: RecordingState,
    pub bundles: Vec<ClapBundle>,
    pub active_imports: Arc<AtomicUsize>,
    /// Current project directory. Set via `AudioCommand::SetProjectDir`
    /// whenever the app opens, creates, or saves-as a project.
    /// Recording and import refuse to run when this is `None`.
    pub project_dir: Option<PathBuf>,
    /// Hardware MIDI state: input/output registries, outbound held
    /// notes, and the device-list caches. Moved out of `HandlerState`
    /// so the half-dozen MIDI-only fields don't crowd the rest of the
    /// engine-thread bookkeeping.
    pub midi_hw: MidiHardwareState,
    /// Per-track recording state for live MIDI. A fresh entry is
    /// created lazily on the first NoteOn for an armed instrument
    /// track during playback; cleared on transport stop.
    pub midi_recording: HashMap<TrackId, RecordingMidiState>,
    /// Live note events parked while a plugin's lock was contended.
    /// Keeps a retried NoteOn ordered ahead of any later NoteOff for
    /// the same key; flushed every engine-loop iteration and before
    /// any direct delivery to the same plugin.
    pub live_note_stash: MidiStash,
    /// MIDI clock master (engine emits clock to a hardware device).
    pub midi_clock_sender: MidiClockSender,
    /// MIDI clock slave (engine receives clock from a hardware device).
    pub midi_clock_receiver: MidiClockReceiver,
    /// Smoothing tempo tracker for incoming clock pulses.
    pub midi_clock_tempo: ClockTempoTracker,
    /// True while an external clock master is currently running
    /// (between Start/Continue and Stop). Used to gate transport
    /// drive: stray clock pulses outside of run state don't trigger
    /// playback.
    pub midi_clock_external_running: bool,
    /// Last BPM emitted to the GUI from the clock tracker, so we
    /// only emit when the value moves perceptibly. Avoids a steady
    /// stream of `MidiClockTempoDetected` events at every pulse.
    pub midi_clock_last_emitted_bpm: f32,
    /// In-flight realtime "bounce in place" run. The engine loop's
    /// poll hook checks the playhead each iteration and, when it
    /// crosses `pending_bounce.stop_at`, pauses the transport,
    /// finalizes the recording, restores the mute snapshot, and emits
    /// `TrackBounceCompleted`. `None` outside of an active bounce.
    pub pending_bounce: Option<super::bounce_realtime::PendingBounce>,
    /// Reference-track (A/B) state: loaded references, active selection,
    /// monitored source, and the loudness-match / trim / loop knobs.
    pub reference: super::reference::ReferencePlayer,
    /// In-flight cycle-record run, or `None` when not loop-recording.
    pub loop_record_session: Option<LoopRecordSession>,
}

/// Hard cap on concurrent clip decode threads. Import commands past this
/// bound get dropped with an error event.
pub(crate) const MAX_CONCURRENT_IMPORTS: usize = 4;

#[allow(clippy::too_many_arguments)]
pub(crate) fn engine_thread(
    cmd_rx: Receiver<AudioCommand>,
    cmd_tx_retry: Sender<AudioCommand>,
    event_tx: Sender<AudioEvent>,
    shared: Arc<SharedState>,
    tracks_arc: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses_arc: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master_arc: Arc<RwLock<MasterBus>>,
    clips_arc: Arc<RwLock<Vec<AudioClip>>>,
    midi_clips_arc: Arc<RwLock<Vec<MidiClip>>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    plugins_arc: Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    latency_comp: Arc<arc_swap::ArcSwap<crate::latency::LatencyComp>>,
    monitor_prod: Arc<Mutex<ringbuf::HeapProd<f32>>>,
    live_midi_tx: Sender<LiveMidiEvent>,
    live_midi_rx: Receiver<LiveMidiEvent>,
    live_control_tx: Sender<LiveControlEvent>,
    live_control_rx: Receiver<LiveControlEvent>,
    clock_tx: Sender<MidiClockEvent>,
    clock_rx: Receiver<MidiClockEvent>,
    sample_rate: u32,
    buf_frames: usize,
    quantum: usize,
) {
    let mut state = HandlerState {
        next_track_id: 1,
        next_bus_id: 1,
        next_clip_id: 1,
        next_asset_id: 1,
        next_plugin_id: 1,
        next_send_id: 1,
        aux_sends: IndexMap::new(),
        next_take_group_id: 1,
        rec: RecordingState::new(sample_rate),
        bundles: Vec::new(),
        active_imports: Arc::new(AtomicUsize::new(0)),
        project_dir: None,
        midi_hw: MidiHardwareState::new(live_midi_tx, live_control_tx),
        midi_recording: HashMap::new(),
        live_note_stash: MidiStash::new(),
        midi_clock_sender: MidiClockSender::new(),
        midi_clock_receiver: MidiClockReceiver::new(clock_tx),
        midi_clock_tempo: ClockTempoTracker::default(),
        midi_clock_external_running: false,
        midi_clock_last_emitted_bpm: 0.0,
        pending_bounce: None,
        reference: super::reference::ReferencePlayer::new(),
        loop_record_session: None,
    };
    let ctx = HandlerCtx {
        shared: &shared,
        tracks: &tracks_arc,
        busses: &busses_arc,
        master: &master_arc,
        clips: &clips_arc,
        midi_clips: &midi_clips_arc,
        plugins: &plugins_arc,
        tempo_map: &tempo_map,
        latency_comp: &latency_comp,
        monitor_prod: &monitor_prod,
        event_tx: &event_tx,
        cmd_tx_retry: &cmd_tx_retry,
        sample_rate,
        buf_frames,
        quantum,
    };

    let mut last_playhead_report = std::time::Instant::now();
    let mut last_audition_report = std::time::Instant::now();
    // Previous-iteration playhead, used to detect a loop wrap so the
    // cycle-record seam handler can roll the just-finished pass into a take.
    let mut last_playhead: SamplePos = 0;

    // Report actual sample rate to GUI
    let _ = ctx
        .event_tx
        .send(AudioEvent::SampleRateDetected { sample_rate });

    // Add a default track
    {
        let id = state.next_track_id;
        let track = Track::new(id, "Track 1".to_string());
        ctx.tracks.write().insert(id, track);
        let _ = ctx.event_tx.send(AudioEvent::TrackAdded { track_id: id });
        state.next_track_id += 1;
    }

    loop {
        match cmd_rx.recv_timeout(std::time::Duration::from_millis(16)) {
            Ok(AudioCommand::ShutDown) => break,
            Ok(cmd) => {
                // Commands that change the track/bus/plugin topology can
                // change per-chain latency; republish the plugin-delay-
                // compensation table after they run. Checked before
                // dispatch because dispatch consumes the command.
                let refresh_latency = plugins::affects_latency(&cmd);
                dispatch(&ctx, &mut state, cmd);
                if refresh_latency {
                    plugins::refresh_latency_comp(&ctx);
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }

        // Drain any hardware MIDI input that's queued up since the
        // previous iteration. Each event is dispatched into the same
        // queue_note_on/off path as `AudioCommand::SendNoteOn`, plus
        // optional record-into-clip and Thru-to-output.
        for ev in live_midi_rx.try_iter() {
            midi::handle_live_midi_event(&ctx, &mut state, ev);
        }

        // Drain control-surface CC/note messages queued since the last
        // iteration. Binding application lands in todo #430; today they
        // are consumed so the bounded channel can't back up.
        for ev in live_control_rx.try_iter() {
            midi::handle_live_control_event(&ctx, &mut state, ev);
        }

        // Retry live note events parked while a plugin lock was
        // contended, so a stashed NoteOn/NoteOff still drains promptly
        // even when no further input arrives for that plugin.
        midi::flush_live_note_stash(&ctx, &mut state);

        // Drain incoming MIDI clock messages and apply them to the
        // transport (Start/Stop/Continue/SongPosition) plus the
        // smoothing tempo tracker.
        for ev in clock_rx.try_iter() {
            midi::handle_midi_clock_event(&ctx, &mut state, ev);
        }

        // Step the timeline → MIDI output bridge: any note whose
        // start/end falls in (last_playhead..current_playhead] gets
        // sent to the configured hardware output. Granularity is
        // engine-thread cadence (~16 ms); precise enough for most
        // hardware-synth use cases and simpler than a lock-free
        // audio→engine queue.
        midi::poll_timeline_to_midi_output(&ctx, &mut state);

        // Emit MIDI clock pulses to a configured master device. Done
        // every iteration so the wire-level clock advances at engine
        // cadence (~60 Hz) rather than only on transport events.
        midi::poll_midi_clock_send(&ctx, &mut state);

        // Advance any pending record count-in: once the playhead
        // catches up to the user's original record-start, the real
        // recording stream opens.
        transport::poll_precount(&ctx, &mut state);

        // Drive an in-flight realtime "bounce in place" run: pauses
        // the transport, restores the mute snapshot, and emits
        // `TrackBounceCompleted` once the playhead crosses the end of
        // the source track's MIDI plus tail.
        bounce_realtime::poll_pending_bounce(&ctx, &mut state);

        // Sync the stable `bpm` field from the tempo event table so
        // the mixer (audio thread) always sees the correct tempo for
        // the current playhead position. Read is wait-free via ArcSwap;
        // only publish a new snapshot when the bpm actually moves.
        {
            let playhead = ctx
                .shared
                .playhead
                .load(std::sync::atomic::Ordering::Relaxed);
            let current = ctx.tempo_map.load();
            if current.sync_bpm_would_change(playhead, ctx.sample_rate) {
                let mut new_tm = (**current).clone();
                new_tm.sync_bpm_at(playhead, ctx.sample_rate);
                ctx.tempo_map.store(Arc::new(new_tm));
            }
        }

        // Drain recording ring buffer into per-track buffers
        if ctx.shared.recording.load(Ordering::Relaxed) {
            state.rec.drain_ring_to_buffers();
        }

        // Audition preview housekeeping: emit AuditionStopped on a natural
        // finish, keep the sync-to-tempo ratio current, and throttle the
        // AuditionPosition events that drive the preview scrub playhead.
        audition::poll_audition(&ctx, &mut last_audition_report);
        // Cycle-record: when the playhead wraps a loop boundary mid-record,
        // roll the just-completed pass into a take and start a fresh one.
        transport::poll_loop_record_seam(&ctx, &mut state, &mut last_playhead);

        // Report playhead position at ~60Hz using wall-clock time
        if ctx.shared.playing.load(Ordering::SeqCst)
            && last_playhead_report.elapsed() >= std::time::Duration::from_millis(16)
        {
            last_playhead_report = std::time::Instant::now();
            let pos = ctx.shared.playhead.load(Ordering::SeqCst);
            let _ = ctx.event_tx.send(AudioEvent::PlayheadMoved(pos));
        }
    }

    // Shutdown ordering: drop every live CLAP plugin instance BEFORE
    // `state` falls out of scope and `state.bundles` (`Vec<ClapBundle>`)
    // dlclose's each `.clap` shared library. `plugins_arc` is shared
    // with the main thread (`Resonance.engine.plugins`) and with the
    // cpal output-stream callback closure — both keep the `Arc` alive
    // past this thread's exit, so the `ClapInstance` values inside
    // wouldn't otherwise drop here. Without this step:
    //   - the audio callback (still running until `_stream` is dropped
    //     during the main thread's `Resonance` teardown) iterates the
    //     map and calls `(*plugin).process` against a now-unloaded
    //     library — segfault on the `cpal_alsa_out` thread; and
    //   - when `Resonance` finally drops, the `Arc` hits refcount 0
    //     on the main thread, every `ClapInstance::drop` runs
    //     `close_gui` / `stop_processing` / `deactivate` / `destroy`
    //     against freed function pointers — segfault on exit.
    // Clearing the map here runs each `ClapInstance::drop` while the
    // libraries are still mapped in. The IndexMap is then empty when
    // bundles unload during `state` drop a few lines down.
    //
    // Pattern mirrors `engine::plugins::handle_remove_plugin`: swap
    // the contents out under the write lock, then drop the swapped-
    // out IndexMap with the lock released so the audio callback's
    // `try_read` isn't held off any longer than the swap itself.
    let drained_plugins: IndexMap<PluginInstanceId, Mutex<SyncClapInstance>> =
        std::mem::take(&mut *plugins_arc.write());
    drop(drained_plugins);
}

/// Route an `ExportAudio` / `BounceToWav` job to the offline renderer.
///
/// Both commands share one render driver, which feeds the rendered mix to
/// the encoder sink selected by `settings.format` (WAV 16/24-bit/f32 or
/// FLAC, with export resampling when the format requests a different
/// sample rate). `reporter` selects the event family: `BounceToWav` keeps
/// the legacy `Bounce*` events (and, with default-WAV settings, byte-for-
/// byte the old output); `ExportAudio` emits the generalized `Export*`
/// events. MP3/Opus sinks and the two-pass loudness stage land in the
/// follow-up todos (#651–#652).
fn dispatch_export(
    ctx: &HandlerCtx,
    path: String,
    settings: ExportSettings,
    reporter: bounce::ExportReporter,
) {
    bounce::export_spawn(
        path,
        settings,
        reporter,
        Arc::clone(ctx.shared),
        Arc::clone(ctx.tracks),
        Arc::clone(ctx.busses),
        Arc::clone(ctx.master),
        Arc::clone(ctx.clips),
        Arc::clone(ctx.midi_clips),
        Arc::clone(ctx.plugins),
        Arc::clone(ctx.tempo_map),
        ctx.sample_rate,
        ctx.event_tx.clone(),
    );
}

fn dispatch(ctx: &HandlerCtx, state: &mut HandlerState, cmd: AudioCommand) {
    match cmd {
        // -- Transport --
        AudioCommand::Play => transport::handle_play(ctx, state),
        AudioCommand::Record { precount_bars } => {
            transport::handle_record(ctx, state, precount_bars)
        }
        AudioCommand::Pause => transport::handle_pause(ctx, state),
        AudioCommand::Stop => transport::handle_stop(ctx, state),
        AudioCommand::SeekTo(pos) => transport::handle_seek_to(ctx, state, pos),
        AudioCommand::SetBpm { bpm } => transport::handle_set_bpm(ctx, bpm),
        AudioCommand::SetTempoEvents { tempo, signature } => {
            transport::handle_set_tempo_events(ctx, tempo, signature)
        }
        AudioCommand::SetTimeSignature {
            numerator,
            denominator,
        } => transport::handle_set_time_signature(ctx, numerator, denominator),
        AudioCommand::SetMetronomeEnabled { enabled } => {
            transport::handle_set_metronome_enabled(ctx, enabled)
        }
        AudioCommand::SetLoopRange {
            enabled,
            loop_in,
            loop_out,
        } => transport::handle_set_loop_range(ctx, state, enabled, loop_in, loop_out),
        AudioCommand::SetLoopRecordMode(on) => transport::handle_set_loop_record_mode(state, on),

        // -- Audio clips --
        AudioCommand::ImportClip {
            track_id,
            path,
            start_sample,
        } => clips::handle_import_clip(ctx, state, track_id, path, start_sample),
        AudioCommand::ImportAudioToPool { paths } => {
            super::import_pool::handle_import_audio_to_pool(ctx, state, paths)
        }
        AudioCommand::MoveClip {
            clip_id,
            new_start_sample,
            new_track_id,
        } => clips::handle_move_clip(ctx, clip_id, new_start_sample, new_track_id),
        AudioCommand::TrimClip {
            clip_id,
            new_start_sample,
            trim_start_frames,
            trim_end_frames,
        } => clips::handle_trim_clip(
            ctx,
            clip_id,
            new_start_sample,
            trim_start_frames,
            trim_end_frames,
        ),
        AudioCommand::DeleteClip { clip_id } => clips::handle_delete_clip(ctx, clip_id),
        AudioCommand::SetClipFade {
            clip_id,
            fade_in_frames,
            fade_in_curve,
            fade_out_frames,
            fade_out_curve,
        } => clips::handle_set_clip_fade(
            ctx,
            clip_id,
            fade_in_frames,
            fade_in_curve,
            fade_out_frames,
            fade_out_curve,
        ),
        AudioCommand::SetClipGain { clip_id, gain_db } => {
            clips::handle_set_clip_gain(ctx, clip_id, gain_db)
        }
        AudioCommand::SetClipWarp {
            clip_id,
            warp_enabled,
            original_bpm,
            transpose_semitones,
            warp_algorithm,
        } => clips::handle_set_clip_warp(
            ctx,
            clip_id,
            warp_enabled,
            original_bpm,
            transpose_semitones,
            warp_algorithm,
        ),
        AudioCommand::SetClipWarpMarkers { clip_id, markers } => {
            clips::handle_set_clip_warp_markers(ctx, clip_id, markers)
        }
        AudioCommand::DetectClipTempo { clip_id } => {
            clips::handle_detect_clip_tempo(ctx, clip_id)
        }
        AudioCommand::AnalyzeClipPitch { clip_id } => {
            vocal_analysis::handle_analyze_clip_pitch(ctx, state, clip_id)
        }
        AudioCommand::SetProjectDir(dir) => {
            state.project_dir = Some(dir);
        }
        AudioCommand::LoadClipFromWav {
            clip_id,
            track_id,
            start_sample,
            path,
            name,
            trim_start_frames,
            trim_end_frames,
        } => clips::handle_load_clip_from_wav(
            ctx,
            state,
            clip_id,
            track_id,
            start_sample,
            path,
            name,
            trim_start_frames,
            trim_end_frames,
        ),
        AudioCommand::SaveClipsToProjectDir => clips::handle_save_clips_to_project_dir(ctx, state),

        // -- Tracks --
        AudioCommand::SetTrackVolume { track_id, volume } => {
            tracks::handle_set_track_volume(ctx, track_id, volume)
        }
        AudioCommand::SetTrackPan { track_id, pan } => {
            tracks::handle_set_track_pan(ctx, track_id, pan)
        }
        AudioCommand::SetTrackMute { track_id, muted } => {
            tracks::handle_set_track_mute(ctx, track_id, muted)
        }
        AudioCommand::SetMasterVolume { volume } => tracks::handle_set_master_volume(ctx, volume),
        AudioCommand::SetTrackSolo { track_id, soloed } => {
            tracks::handle_set_track_solo(ctx, track_id, soloed)
        }
        AudioCommand::AddTrack { id_hint, name } => {
            tracks::handle_add_track(ctx, state, id_hint, name)
        }
        AudioCommand::CreateSubTrack {
            sub_id,
            parent_track_id,
            output_port_index,
            name,
        } => tracks::handle_create_sub_track(
            ctx,
            state,
            sub_id,
            parent_track_id,
            output_port_index,
            name,
        ),
        AudioCommand::RemoveTrack { track_id } => tracks::handle_remove_track(ctx, state, track_id),
        AudioCommand::SetTrackRecordArm { track_id, armed } => {
            tracks::handle_set_track_record_arm(ctx, track_id, armed)
        }
        AudioCommand::SetTrackMono { track_id, mono } => {
            tracks::handle_set_track_mono(ctx, state, track_id, mono)
        }
        AudioCommand::SetTrackMonitor { track_id, enabled } => {
            tracks::handle_set_track_monitor(ctx, state, track_id, enabled)
        }
        AudioCommand::SetTrackInputDevice {
            track_id,
            device_name,
        } => tracks::handle_set_track_input_device(ctx, state, track_id, device_name),
        AudioCommand::SetTrackInputPort {
            track_id,
            port_index,
        } => tracks::handle_set_track_input_port(ctx, state, track_id, port_index),
        AudioCommand::ListInputDevices => tracks::handle_list_input_devices(ctx),
        AudioCommand::ClearAll => tracks::handle_clear_all(ctx, state),

        // -- Plugins --
        AudioCommand::AddPlugin {
            track_id,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        } => plugins::handle_add_plugin(
            ctx,
            state,
            track_id,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        ),
        AudioCommand::RemovePlugin {
            track_id,
            instance_id,
        } => plugins::handle_remove_plugin(ctx, track_id, instance_id),
        AudioCommand::ScanPlugins => {
            scan::scan_plugins(ctx.plugins, ctx.tracks, &mut state.bundles, ctx.event_tx)
        }
        AudioCommand::SetPluginParam {
            instance_id,
            param_id,
            value,
        } => plugins::handle_set_plugin_param(ctx, instance_id, param_id, value),
        AudioCommand::OpenPluginEditor { instance_id } => {
            plugins::handle_open_plugin_editor(ctx, instance_id)
        }
        AudioCommand::ClosePluginEditor { instance_id } => {
            plugins::handle_close_plugin_editor(ctx, instance_id)
        }
        AudioCommand::SavePluginState { instance_id } => {
            plugins::handle_save_plugin_state(ctx, instance_id)
        }
        AudioCommand::LoadPluginState { instance_id, data } => {
            plugins::handle_load_plugin_state(ctx, instance_id, data)
        }
        AudioCommand::SaveAllPluginStates => plugins::handle_save_all_plugin_states(ctx),

        // -- Bounce / export --
        // Legacy WAV bounce: a thin shim over the generalized export
        // path with default 32-bit-float WAV settings (doc #196).
        AudioCommand::BounceToWav { path } => dispatch_export(
            ctx,
            path,
            ExportSettings::default_wav(),
            bounce::ExportReporter::Bounce,
        ),
        AudioCommand::ExportAudio { path, settings } => {
            dispatch_export(ctx, path, settings, bounce::ExportReporter::Export)
        }
        AudioCommand::BounceTrackToAudio {
            source_track_id,
            target_track_id,
            target_clip_id,
            name,
        } => bounce::to_audio_clip_spawn(
            source_track_id,
            target_track_id,
            target_clip_id,
            name,
            Arc::clone(ctx.shared),
            Arc::clone(ctx.tracks),
            Arc::clone(ctx.busses),
            Arc::clone(ctx.master),
            Arc::clone(ctx.clips),
            Arc::clone(ctx.midi_clips),
            Arc::clone(ctx.plugins),
            Arc::clone(ctx.tempo_map),
            ctx.sample_rate,
            ctx.event_tx.clone(),
        ),
        AudioCommand::BounceTrackRealtimeToAudio {
            source_track_id,
            target_track_id,
            input_device_name,
            input_port_index,
            mono,
        } => bounce_realtime::handle_bounce_track_realtime(
            ctx,
            state,
            source_track_id,
            target_track_id,
            input_device_name,
            input_port_index,
            mono,
        ),
        AudioCommand::CancelBounce => {
            // Set the cooperative cancel flag for both bounce paths.
            // The offline renderers run on worker threads and poll the
            // flag between chunks; the realtime path picks it up on the
            // next engine-loop iteration via `poll_pending_bounce`.
            ctx.shared
                .bounce_cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        AudioCommand::ExportStems {
            targets,
            range,
            sample_rate,
            bit_depth,
            include_fx_tail,
        } => bounce::export_stems_spawn(
            targets,
            range,
            sample_rate,
            bit_depth,
            include_fx_tail,
            Arc::clone(ctx.shared),
            Arc::clone(ctx.tracks),
            Arc::clone(ctx.busses),
            Arc::clone(ctx.master),
            Arc::clone(ctx.clips),
            Arc::clone(ctx.midi_clips),
            Arc::clone(ctx.plugins),
            Arc::clone(ctx.tempo_map),
            ctx.sample_rate,
            ctx.event_tx.clone(),
        ),
        AudioCommand::CancelStemExport => {
            // Shares the cooperative cancel flag with the bounce paths;
            // the stem-export worker polls it between targets.
            ctx.shared
                .bounce_cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }

        // -- Freeze --
        AudioCommand::FreezeTrack {
            track_id,
            cache_path,
        } => bounce::to_freeze_cache_spawn(
            track_id,
            cache_path,
            Arc::clone(ctx.shared),
            Arc::clone(ctx.tracks),
            Arc::clone(ctx.busses),
            Arc::clone(ctx.master),
            Arc::clone(ctx.clips),
            Arc::clone(ctx.midi_clips),
            Arc::clone(ctx.plugins),
            Arc::clone(ctx.tempo_map),
            ctx.sample_rate,
            ctx.event_tx.clone(),
        ),
        AudioCommand::SetTrackFrozenSource { track_id, source } => {
            tracks::handle_set_track_frozen_source(ctx, track_id, source)
        }
        AudioCommand::UnfreezeTrack { track_id } => {
            tracks::handle_unfreeze_track(ctx, track_id)
        }
        AudioCommand::CancelFreeze => {
            // Freeze reuses the shared bounce-cancel atomic (the offline
            // freeze renderer polls it between chunks, same as the bounce
            // renderers). The worker drops the partial cache file and
            // emits `FreezeCancelled`.
            ctx.shared
                .bounce_cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }

        // -- Instrument tracks + MIDI --
        AudioCommand::AddInstrumentTrack { id_hint, name } => {
            midi::handle_add_instrument_track(ctx, state, id_hint, name)
        }
        AudioCommand::AddVocalTrack { id_hint, name } => {
            midi::handle_add_vocal_track(ctx, state, id_hint, name)
        }
        AudioCommand::CreateMidiClip {
            track_id,
            start_sample,
            duration_ticks,
            name,
        } => {
            midi::handle_create_midi_clip(ctx, state, track_id, start_sample, duration_ticks, name)
        }
        AudioCommand::LoadMidiClipDirect {
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            notes,
            name,
            trim_start_ticks,
            trim_end_ticks,
        } => midi::handle_load_midi_clip_direct(
            ctx,
            state,
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            notes,
            name,
            trim_start_ticks,
            trim_end_ticks,
        ),
        AudioCommand::MoveMidiClip {
            clip_id,
            new_start_sample,
            new_track_id,
        } => midi::handle_move_midi_clip(ctx, clip_id, new_start_sample, new_track_id),
        AudioCommand::TrimMidiClip {
            clip_id,
            new_start_sample,
            trim_start_ticks,
            trim_end_ticks,
        } => midi::handle_trim_midi_clip(
            ctx,
            clip_id,
            new_start_sample,
            trim_start_ticks,
            trim_end_ticks,
        ),
        AudioCommand::DeleteMidiClip { clip_id } => midi::handle_delete_midi_clip(ctx, clip_id),
        AudioCommand::AddMidiNote { clip_id, note } => {
            midi::handle_add_midi_note(ctx, clip_id, note)
        }
        AudioCommand::RemoveMidiNote {
            clip_id,
            note_index,
        } => midi::handle_remove_midi_note(ctx, clip_id, note_index),
        AudioCommand::MoveMidiNote {
            clip_id,
            note_index,
            new_start_tick,
            new_note,
        } => midi::handle_move_midi_note(ctx, clip_id, note_index, new_start_tick, new_note),
        AudioCommand::ResizeMidiNote {
            clip_id,
            note_index,
            new_duration_ticks,
        } => midi::handle_resize_midi_note(ctx, clip_id, note_index, new_duration_ticks),
        AudioCommand::SetMidiNoteVelocity {
            clip_id,
            note_index,
            velocity,
        } => midi::handle_set_midi_note_velocity(ctx, clip_id, note_index, velocity),
        // GUI-originated notes carry no arrival timestamp; offset 0
        // (start of the next block) is the earliest delivery anyway.
        AudioCommand::SendNoteOn {
            track_id,
            note,
            velocity,
        } => midi::handle_send_note_on(ctx, state, track_id, note, velocity, 0),
        AudioCommand::SendNoteOff { track_id, note } => {
            midi::handle_send_note_off(ctx, state, track_id, note, 0)
        }
        AudioCommand::ListMidiInputDevices => midi::handle_list_midi_inputs(ctx, state),
        AudioCommand::ListMidiOutputDevices => midi::handle_list_midi_outputs(ctx, state),
        AudioCommand::SetTrackMidiInput {
            track_id,
            device,
            channel,
        } => midi::handle_set_track_midi_input(ctx, state, track_id, device, channel),
        AudioCommand::SetTrackMidiOutput {
            track_id,
            device,
            channel,
        } => midi::handle_set_track_midi_output(ctx, state, track_id, device, channel),
        AudioCommand::SetMidiClockOutput { device, enabled } => {
            midi::handle_set_midi_clock_output(ctx, state, device, enabled)
        }
        AudioCommand::SetMidiClockInput { device, enabled } => {
            midi::handle_set_midi_clock_input(ctx, state, device, enabled)
        }

        // -- Busses --
        AudioCommand::AddBus { id_hint, name } => busses::handle_add_bus(ctx, state, id_hint, name),
        AudioCommand::RemoveBus { bus_id } => busses::handle_remove_bus(ctx, bus_id),
        AudioCommand::SetBusVolume { bus_id, volume } => {
            busses::handle_set_bus_volume(ctx, bus_id, volume)
        }
        AudioCommand::SetBusPan { bus_id, pan } => busses::handle_set_bus_pan(ctx, bus_id, pan),
        AudioCommand::SetBusMute { bus_id, muted } => {
            busses::handle_set_bus_mute(ctx, bus_id, muted)
        }
        AudioCommand::SetBusName { bus_id, name } => busses::handle_set_bus_name(ctx, bus_id, name),
        AudioCommand::SetTrackOutput { track_id, output } => {
            busses::handle_set_track_output(ctx, track_id, output)
        }
        AudioCommand::AddPluginToBus {
            bus_id,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        } => busses::handle_add_plugin_to_bus(
            ctx,
            state,
            bus_id,
            clap_file_path,
            clap_plugin_id,
            id_hint,
        ),
        AudioCommand::RemovePluginFromBus {
            bus_id,
            instance_id,
        } => busses::handle_remove_plugin_from_bus(ctx, bus_id, instance_id),

        // -- Aux sends + return busses --
        AudioCommand::SetBusRole { bus_id, is_return } => {
            busses::handle_set_bus_role(ctx, bus_id, is_return)
        }
        AudioCommand::SetAuxSend {
            id_hint,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        } => busses::handle_set_aux_send(
            ctx, state, id_hint, source, dest, level_db, pre_fader, enabled,
        ),
        AudioCommand::RemoveAuxSend { send_id } => {
            busses::handle_remove_aux_send(ctx, state, send_id)
        }

        // -- Master FX chain + bypass --
        AudioCommand::AddPluginToMaster {
            clap_file_path,
            clap_plugin_id,
            id_hint,
        } => {
            master::handle_add_plugin_to_master(ctx, state, clap_file_path, clap_plugin_id, id_hint)
        }
        AudioCommand::RemovePluginFromMaster { instance_id } => {
            master::handle_remove_plugin_from_master(ctx, instance_id)
        }
        AudioCommand::SetTrackFxBypass { track_id, bypassed } => {
            tracks::handle_set_track_fx_bypass(ctx, track_id, bypassed)
        }
        AudioCommand::SetBusFxBypass { bus_id, bypassed } => {
            busses::handle_set_bus_fx_bypass(ctx, bus_id, bypassed)
        }
        AudioCommand::SetMasterFxBypass { bypassed } => {
            master::handle_set_master_fx_bypass(ctx, bypassed)
        }
        AudioCommand::AuditionFile { path, start_frame } => {
            audition::handle_audition_file(ctx, path, start_frame)
        }
        AudioCommand::StopAudition => {
            if audition::stop_audition_in_place(ctx.shared) {
                let _ = ctx.event_tx.send(AudioEvent::AuditionStopped);
            }
        }
        AudioCommand::SetAuditionOptions {
            loop_enabled,
            sync_to_tempo,
        } => {
            let bpm = ctx.tempo_map.load().bpm as f64;
            audition::set_audition_options_in_place(ctx.shared, bpm, loop_enabled, sync_to_tempo);
        }
        // -- MIDI Learn & hardware controller mapping (doc #167 §2 E2) --
        AudioCommand::SetMidiBinding { binding } => {
            midi_map::handle_set_midi_binding(ctx, binding)
        }
        AudioCommand::ClearMidiBinding { id } => midi_map::handle_clear_midi_binding(ctx, id),
        AudioCommand::SetControllerMap { map } => midi_map::handle_set_controller_map(ctx, map),
        AudioCommand::ClearAllMidiBindings => midi_map::handle_clear_all_midi_bindings(ctx),
        AudioCommand::SetControlSurfaceInput { device } => {
            midi_map::handle_set_control_surface_input(ctx, device)
        }
        AudioCommand::EnterMidiLearn { target } => midi_map::handle_enter_midi_learn(ctx, target),
        AudioCommand::CancelMidiLearn => midi_map::handle_cancel_midi_learn(ctx),

        AudioCommand::PollPeaks => handle_poll_peaks(ctx),

        // -- Reference track (A/B) --
        AudioCommand::LoadReferenceTrack { id_hint, path } => {
            reference::handle_load_reference_track(
                &mut state.reference,
                ctx.event_tx,
                ctx.cmd_tx_retry,
                ctx.sample_rate,
                id_hint,
                path,
            )
        }
        AudioCommand::ReferenceAnalyzed {
            id,
            pcm,
            integrated_lufs,
        } => {
            reference::handle_reference_analyzed(&mut state.reference, id, pcm, integrated_lufs);
            // Decoded PCM just arrived: republish (cursor synced so the
            // active reference starts from the top of its decoded buffer).
            state.reference.publish(&ctx.shared.reference, true);
        }
        AudioCommand::RemoveReferenceTrack { id } => {
            reference::handle_remove_reference_track(&mut state.reference, ctx.event_tx, id);
            // May have cleared the active selection — drop the monitor PCM.
            state.reference.publish(&ctx.shared.reference, false);
        }
        AudioCommand::SetActiveReference { id } => {
            reference::handle_set_active_reference(&mut state.reference, ctx.event_tx, id);
            // New active reference: swap PCM + restart from its cursor.
            state.reference.publish(&ctx.shared.reference, true);
        }
        AudioCommand::SetABSource { source } => {
            reference::handle_set_ab_source(&mut state.reference, ctx.event_tx, source);
            state.reference.publish(&ctx.shared.reference, false);
        }
        AudioCommand::SetRefLoudnessMatch { enabled } => {
            reference::handle_set_ref_loudness_match(&mut state.reference, ctx.event_tx, enabled);
            state.reference.publish(&ctx.shared.reference, false);
        }
        AudioCommand::SetRefTrim { db } => {
            reference::handle_set_ref_trim(&mut state.reference, ctx.event_tx, db);
            state.reference.publish(&ctx.shared.reference, false);
        }
        AudioCommand::AddRefMarker {
            ref_id,
            position_samples,
            label,
        } => reference::handle_add_ref_marker(
            &mut state.reference,
            ctx.event_tx,
            ref_id,
            position_samples,
            label,
        ),
        AudioCommand::RemoveRefMarker { ref_id, marker_id } => {
            reference::handle_remove_ref_marker(&mut state.reference, ctx.event_tx, ref_id, marker_id)
        }
        AudioCommand::SetRefPosition {
            ref_id,
            position_samples,
        } => {
            reference::handle_set_ref_position(
                &mut state.reference,
                ctx.event_tx,
                ref_id,
                position_samples,
            );
            // Explicit scrub: re-sync the live cursor to the new position.
            state.reference.publish(&ctx.shared.reference, true);
        }
        AudioCommand::SetRefLoopToMix { enabled } => {
            reference::handle_set_ref_loop_to_mix(&mut state.reference, ctx.event_tx, enabled);
            state.reference.publish(&ctx.shared.reference, false);
        }
        AudioCommand::PollABMeters => reference::handle_poll_ab_meters(
            &state.reference,
            ctx.shared.mix_meter.load(),
            ctx.shared.ref_meter.load(),
            ctx.event_tx,
        ),

        AudioCommand::ShutDown => {
            // Handled in the engine_thread loop directly; this arm is
            // unreachable in practice but keeps the match exhaustive.
        }
    }
}

/// Snapshot and clear every peak meter (per-track, per-bus, master L/R)
/// and dispatch a `PeakSnapshot` event. Runs on the engine thread, so the
/// `try_read` calls compete only with the audio callback's brief
/// `try_read` — same window as the old direct getter but now off the GUI
/// thread, and the GUI side reads its result via the regular event queue.
fn handle_poll_peaks(ctx: &HandlerCtx) {
    use std::sync::atomic::Ordering;

    let track_peaks = ctx
        .tracks
        .try_read()
        .map(|guard| {
            guard
                .values()
                .map(|t| (t.id, t.swap_peak_l(), t.swap_peak_r()))
                .collect()
        })
        .unwrap_or_default();
    let bus_peaks = ctx
        .busses
        .try_read()
        .map(|guard| {
            guard
                .values()
                .map(|b| (b.id, b.swap_peak_l(), b.swap_peak_r()))
                .collect()
        })
        .unwrap_or_default();
    let master_peak_l = f32::from_bits(ctx.shared.master_peak_l_bits.swap(0, Ordering::AcqRel));
    let master_peak_r = f32::from_bits(ctx.shared.master_peak_r_bits.swap(0, Ordering::AcqRel));
    let _ = ctx.event_tx.send(AudioEvent::PeakSnapshot {
        track_peaks,
        bus_peaks,
        master_peak_l,
        master_peak_r,
    });
}
