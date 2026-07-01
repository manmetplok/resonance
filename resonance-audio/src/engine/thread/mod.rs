//! Engine control thread: owns the command/event loop and the
//! per-command handler dispatch. All mutable engine state that must
//! outlive a single command lives in [`HandlerState`]; the shared
//! references to `Arc<RwLock<...>>` project state, the event sender, and
//! the retry-command sender live in [`HandlerCtx`].
//!
//! Handlers are free functions in the submodules (`transport`, `tracks`,
//! `clips`, `midi`, `plugins`, `busses`). They take `&HandlerCtx` +
//! `&mut HandlerState` + the command payload and execute synchronously.
//!
//! Command dispatch is split into per-category sub-dispatchers in the
//! [`dispatch`] submodule. The top-level [`dispatch::dispatch`] routes
//! each `AudioCommand` to the appropriate category handler.

mod dispatch;

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
    audition, automation, bounce_realtime, external_instrument, midi, plugins, transport,
    SharedState,
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
    /// Parameter-automation lanes, one per [`AutomationTarget`]. Held
    /// engine-thread-local; written by the `SetAutomationLane` /
    /// `ClearAutomationLane` / `SetAutomationReadEnabled` handlers.
    /// Points are kept sorted so a later per-block evaluator can sample
    /// without sorting or allocating. No audio is applied yet.
    pub automation_lanes: automation::AutomationLanes,
    /// External-instrument configs, one per track. Held engine-thread-local;
    /// written by the `SetExternalInstrument` / `ClearExternalInstrument` /
    /// `SetExternalInstrumentPatch` / `SetExternalInstrumentLatencyOffset`
    /// handlers. The presence of an entry marks the track as an external
    /// instrument; device/channel/monitor/arm live on the `Track` itself.
    pub external_instruments: external_instrument::ExternalInstruments,
    /// In-flight round-trip latency auto-detect ("ping"). Owns its own capture
    /// stream + ring for the duration of the measurement so it never touches
    /// the recording session. The engine loop's `poll_pending_latency_ping`
    /// hook drains it each iteration, detects the returned impulse, applies the
    /// measured offset, and republishes PDC — or reports a clean failure once
    /// the listen window elapses. `None` outside of an active ping.
    pub pending_latency_ping: Option<super::external_instrument_ping::PendingLatencyPing>,
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
        automation_lanes: automation::AutomationLanes::new(),
        external_instruments: external_instrument::ExternalInstruments::new(),
        pending_latency_ping: None,
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
                dispatch::dispatch(&ctx, &mut state, cmd);
                if refresh_latency {
                    plugins::refresh_latency_comp(&ctx, &state.external_instruments);
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

        // Advance an in-flight external-instrument latency ping: drain the
        // capture ring, detect the returned impulse, and apply the measured
        // round-trip offset — or report a clean failure once the listen
        // window elapses.
        super::external_instrument_ping::poll_pending_latency_ping(&ctx, &mut state);

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
