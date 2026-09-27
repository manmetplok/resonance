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
pub(crate) mod test_support;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use crate::clap_host::{ClapBundle, PluginMap};
use crate::midi_clock::{
    ClockTempoTracker, MidiClockEvent, MidiClockReceiver, MidiClockSender,
};
use crate::midi_hardware::{LiveControlEvent, LiveMidiEvent};
use crate::mixer::MidiStash;
use crate::recording::RecordingState;
use crate::types::*;
use resonance_common::{TakeGroup, TakeGroupId, TimelineRange};

use super::import_queue::ImportQueue;
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
    pub clips: &'a Arc<RwLock<Vec<AudioClip>>>,
    pub plugins: &'a Arc<RwLock<PluginMap>>,
    pub tempo_map: &'a Arc<arc_swap::ArcSwap<TempoMap>>,
    pub latency_comp: &'a Arc<arc_swap::ArcSwap<crate::latency::LatencyComp>>,
    /// Parameter-automation snapshot published to the audio callback and
    /// the offline bounce. Rebuilt from `automation_lanes` whenever the
    /// lane set changes.
    pub automation: &'a Arc<arc_swap::ArcSwap<automation::AutomationSnapshot>>,
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
}

/// Mutable engine-thread-local state that persists across command
/// dispatches: monotonic id counters, the recording session, the loaded
/// CLAP bundles, and the bounded clip-import worker pool.
pub(crate) struct HandlerState {
    pub next_clip_id: ClipId,
    /// Aux sends keyed by id, in insertion order. Engine-thread-local
    /// (never read from the audio callback), so plain data — see
    /// [`AuxSend`]. The source of truth for cyclic-route validation.
    pub aux_sends: IndexMap<SendId, AuxSend>,
    /// External sidechain key routes, one per plugin instance. Mirrored
    /// to the audio thread by `engine::sidechain::publish`.
    pub sidechain_routes: crate::engine::sidechain::SidechainRoutes,
    /// Monotonic id allocator for cycle-record take groups. Consumed only
    /// by `transport::loop_record_group_for`, and only when a run finds no
    /// existing lane for its track + loop region: one lane per slot, so
    /// repeated runs over the same region reuse the group already in
    /// [`HandlerState::take_groups`] (todo #1392).
    pub next_take_group_id: TakeGroupId,
    pub rec: RecordingState,
    pub bundles: Vec<ClapBundle>,
    /// Bounded worker pool for clip import / project-load WAV work. The
    /// engine thread only ever enqueues onto it (see
    /// [`ImportQueue::submit`]); the heavy mmap + waveform decimation
    /// runs on its workers. Dropped with `HandlerState` at engine
    /// shutdown, which lets the workers finish and exit.
    pub imports: ImportQueue,
    /// Bumped by every `ClearAll`. `ImportAudioToPool`'s worker, a track
    /// freeze's, and `LoadClipFromWav`/`LoadTakeClipFromWav`'s
    /// (`clips::submit_clip_load`, FU-D7c) all capture it when queued and
    /// drop their result if it changed meanwhile, so a load or import
    /// never lands in the project that replaced its own (code review
    /// UPD-09). Shared with the import workers.
    pub clear_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// The `SetProjectDir` clip-id reservation scan still running on its
    /// worker (FU-M12b); see `clips::settle_clip_id_scan`.
    pub clip_id_scan: Option<std::thread::JoinHandle<Option<ClipId>>>,
    /// Freeze caches being converted to the engine rate on a worker
    /// (FU-A4c); see `tracks::settle_frozen_conversions`.
    pub frozen_conversions: Vec<crate::engine::tracks::PendingFrozenConversion>,
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
    /// Authoritative take groups keyed by id (epic #15, doc #165).
    /// Populated as cycle-record passes are captured
    /// (`transport::finalize_loop_record_pass`) and edited by the
    /// `SetTakeComp` / `SetActiveTake` handlers in
    /// [`crate::engine::takes`]. Every mutation republishes the flattened
    /// `SharedState::take_comp` table, which is what keeps comp playback
    /// and comp bounce in step.
    pub take_groups: HashMap<TakeGroupId, TakeGroup>,
    /// Recordings of takes that have been removed (`RemoveTake` /
    /// `RemoveTakeGroup`, ba todo #1397), held out of the shared clip list
    /// so they cannot sound.
    ///
    /// Parked rather than dropped, and never deleted from disk: a removal
    /// is undoable, and the `RestoreTakeGroups` an undo sends carries the
    /// take's `clip_ref` back — so the clip has to still be here for the
    /// restored take to be audible as well as visible. Session-local, like
    /// the id allocators beside it: `ClearAll` empties it, and a project
    /// reload starts from an empty park with the orphaned WAV left on disk.
    ///
    /// Behind an `Arc` — the only field here that is not purely
    /// engine-thread-local — because the clip-load worker has to be able to
    /// see it (ba todo #1403): a removal racing a `LoadTakeClipFromWav`
    /// that is still in flight parks a *claim*, and the worker delivers the
    /// finished clip into the park rather than into the render's input.
    /// [`TakeClipPark`](crate::engine::take_park::TakeClipPark) states the
    /// locking contract that keeps the two in step.
    pub take_clip_park: Arc<crate::engine::take_park::TakeClipPark>,
    /// The live ticket of every audio-clip load still on a worker
    /// (code review FU-A13e): `DeleteClip` withdraws an id's ticket so a
    /// load submitted before it never publishes, whatever order the
    /// workers finish in. Shared with the load workers, like
    /// [`Self::take_clip_park`]; see
    /// [`ClipLoadTickets`](crate::engine::clip_loads::ClipLoadTickets).
    pub clip_load_tickets: Arc<crate::engine::clip_loads::ClipLoadTickets>,
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
    /// Clip edits that arrived before their clip existed (ba doc #276
    /// BUG 1).
    ///
    /// `LoadClipFromWav` hands the mmap + waveform work to a worker
    /// thread, so a clip is not in `ctx.clips` when the command that
    /// created it returns. A client that places a clip and immediately
    /// trims it — the only way to make one asset serve several sections,
    /// and what the whole arrangement flow does — used to hit the
    /// handlers' "missing lookup ⇒ no-op" convention: the trim vanished,
    /// the app mirror kept the geometry it had optimistically applied,
    /// and every reported number stayed right while the render played
    /// the untrimmed source.
    ///
    /// Such a command is parked here instead and retried by
    /// [`super::clips::poll_deferred_clip_commands`] on each engine-loop
    /// iteration.
    pub deferred_clip_commands: Vec<super::clips::DeferredClipCommand>,
    /// Cancel token of the render most recently started by a bounce /
    /// export command (`BounceToWav` / `ExportAudio` / `BounceTrackToAudio`
    /// / `BounceTrackRealtimeToAudio`); `CancelBounce` flips it. Every
    /// render gets a FRESH token at start, so a cancel aimed at one render
    /// can neither abort a different renderer that happens to poll first
    /// nor be lost to a later render clearing a shared flag — the two
    /// failure modes of the old single `SharedState::bounce_cancel` atomic.
    /// Stays `Some` after the render ends; setting a finished render's
    /// token is harmless.
    pub bounce_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Cancel token of the most recently started freeze render
    /// (`FreezeTrack`); `CancelFreeze` flips it. Same per-render
    /// semantics as [`Self::bounce_cancel`].
    pub freeze_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Cancel token of the most recently started stem export
    /// (`ExportStems`); `CancelStemExport` flips it. Same per-render
    /// semantics as [`Self::bounce_cancel`].
    pub stem_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl HandlerState {
    /// The engine control thread's starting state: every id allocator at
    /// 1, every store empty, no recording or bounce in flight.
    ///
    /// Split out of [`engine_thread`] so the headless harness in
    /// [`test_support`] starts from the *same* state the real thread
    /// does. A second, hand-written copy of this field list would drift
    /// the moment a field gained a non-default starting value, and a
    /// harness that starts from a state the engine never has proves
    /// nothing about the engine.
    pub(crate) fn new(
        sample_rate: u32,
        live_midi_tx: Sender<LiveMidiEvent>,
        live_control_tx: Sender<LiveControlEvent>,
        clock_tx: Sender<MidiClockEvent>,
    ) -> Self {
        Self {
            next_clip_id: 1,
            aux_sends: IndexMap::new(),
            sidechain_routes: Default::default(),
            next_take_group_id: 1,
            rec: RecordingState::new(sample_rate),
            bundles: Vec::new(),
            imports: ImportQueue::default(),
            clear_generation: Default::default(),
            clip_id_scan: None,
            frozen_conversions: Vec::new(),
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
            take_groups: HashMap::new(),
            take_clip_park: Arc::new(crate::engine::take_park::TakeClipPark::default()),
            clip_load_tickets: Arc::default(),
            automation_lanes: automation::AutomationLanes::new(),
            external_instruments: external_instrument::ExternalInstruments::new(),
            pending_latency_ping: None,
            deferred_clip_commands: Vec::new(),
            bounce_cancel: None,
            freeze_cancel: None,
            stem_cancel: None,
        }
    }
}

/// Rebuild the audio-thread automation snapshot from the engine-thread
/// lane map and publish it wait-free. Called after any lane mutation so
/// the audio callback and bounce see the new lanes on their next block.
pub(crate) fn publish_automation_snapshot(
    ctx: &HandlerCtx,
    lanes: &automation::AutomationLanes,
) {
    let snapshot = automation::AutomationSnapshot::build(lanes, &ctx.plugins.read());
    super::retire::publish(ctx.automation, Arc::new(snapshot), &ctx.shared.retired);
}

/// Construction parameters for [`engine_thread`].
///
/// A plain positional parameter list here used to run to 22 arguments,
/// several sharing a type (three `Sender`s, five `Arc<RwLock<…>>>`s) —
/// nothing stopped two same-typed arguments from being passed in the
/// wrong order at the call site; the compiler can't catch a transposed
/// pair when both sides typecheck. Field-name construction makes that
/// class of mistake impossible: every value is bound to its parameter
/// name at the call site, not its position. Field names mirror the
/// removed parameter names 1:1, so a diff against the old signature
/// reads straight across.
pub(crate) struct EngineThreadParams {
    pub cmd_rx: Receiver<AudioCommand>,
    pub cmd_tx_retry: Sender<AudioCommand>,
    pub event_tx: Sender<AudioEvent>,
    pub shared: Arc<SharedState>,
    pub tracks_arc: Arc<RwLock<IndexMap<TrackId, Track>>>,
    pub clips_arc: Arc<RwLock<Vec<AudioClip>>>,
    pub tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    pub plugins_arc: Arc<RwLock<PluginMap>>,
    pub latency_comp: Arc<arc_swap::ArcSwap<crate::latency::LatencyComp>>,
    pub automation: Arc<arc_swap::ArcSwap<automation::AutomationSnapshot>>,
    pub monitor_prod: Arc<Mutex<ringbuf::HeapProd<f32>>>,
    pub live_midi_tx: Sender<LiveMidiEvent>,
    // Events already picked up (and instrument-delivered) by the audio
    // callback, forwarded here for recording + MIDI-thru bookkeeping
    // (doc #260 finding #16).
    pub live_midi_fwd_rx: Receiver<LiveMidiEvent>,
    pub live_control_tx: Sender<LiveControlEvent>,
    pub live_control_rx: Receiver<LiveControlEvent>,
    pub clock_tx: Sender<MidiClockEvent>,
    pub clock_rx: Receiver<MidiClockEvent>,
    pub sample_rate: u32,
    pub buf_frames: usize,
    pub quantum: usize,
}

pub(crate) fn engine_thread(params: EngineThreadParams) {
    let EngineThreadParams {
        cmd_rx,
        cmd_tx_retry,
        event_tx,
        shared,
        tracks_arc,
        clips_arc,
        tempo_map,
        plugins_arc,
        latency_comp,
        automation,
        monitor_prod,
        live_midi_tx,
        live_midi_fwd_rx,
        live_control_tx,
        live_control_rx,
        clock_tx,
        clock_rx,
        sample_rate,
        buf_frames,
        quantum,
    } = params;
    let mut state = HandlerState::new(sample_rate, live_midi_tx, live_control_tx, clock_tx);
    let ctx = HandlerCtx {
        shared: &shared,
        tracks: &tracks_arc,
        clips: &clips_arc,
        plugins: &plugins_arc,
        tempo_map: &tempo_map,
        latency_comp: &latency_comp,
        automation: &automation,
        monitor_prod: &monitor_prod,
        event_tx: &event_tx,
        cmd_tx_retry: &cmd_tx_retry,
        sample_rate,
        buf_frames,
        quantum,
    };

    let mut last_playhead_report = std::time::Instant::now();
    let mut last_audition_report = std::time::Instant::now();
    // Sequence of the last DSP-load report printed (see `cycle_load`).
    let mut cycle_report_seen = 0u64;
    // Coalesce cpal stream underruns into one line per
    // `UNDERRUN_REPORT_INTERVAL` (see `stream_errors`); the counts come
    // from the streams' error callbacks via `SharedState` (FU-H6b).
    let output_underruns = crate::stream_errors::UnderrunRateLimiter::new();
    let input_underruns = crate::stream_errors::UnderrunRateLimiter::new();
    // Live automated-value emission (todo #377): throttle clock + the
    // per-target "last value sent" memo. Reset whenever the transport
    // isn't rolling so a fresh play re-tints the controls.
    let mut last_automated_value_emit = std::time::Instant::now();
    let mut live_value_emitter = automation::LiveValueEmitter::default();
    // Previous-iteration playhead, used to detect a loop wrap so the
    // cycle-record seam handler can roll the just-finished pass into a take.
    let mut last_playhead: SamplePos = 0;

    // Report actual sample rate to GUI
    let _ = ctx
        .event_tx
        .send(AudioEvent::SampleRateDetected { sample_rate });

    // No default track is created here any more (FU-D4a). Until this fix
    // this block unprompted-inserted a literal id-1 "Track 1" right here,
    // before the command loop below ever read from `cmd_rx` — reasoning
    // that it could never collide with anything since nothing had run yet
    // (ARCH-04 D-4's note: there is no `next_track_id` counter left on
    // this side to draw the id from, so it was a literal, the same way a
    // hand-built fixture like `demo::seed_demo_content` picks its own ids
    // outright). That reasoning missed the app's OWN counter, which also
    // starts at 1: a GUI "Add Track" handled before the app had mirrored
    // this unprompted `TrackAdded` echo called `allocate_track_id`, got
    // id 1 too, and this thread refused the resulting `AddTrack` as a
    // collision with the track it had already silently created — a click
    // that visibly did nothing but raise an error banner. Since ARCH-04
    // D-4 the app is the only track-id allocator (`state/ids.rs`); the
    // fresh-session default track is now created the same way any other
    // track is, from an `AddTrack` the app sends itself
    // (`Resonance::send_startup_default_track`), synchronously, before
    // its own event loop can run a second allocation — so nothing can
    // ever race it for id 1 again.
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

        // Service plugin-initiated `clap_host_latency.changed()` /
        // `request_restart()` callbacks: cycle the flagged instances'
        // activation (the safe point at which latency may change),
        // re-read their latency, and republish PDC if anything moved
        // (doc #260 finding #10).
        plugins::poll_plugin_host_requests(&ctx, &state.external_instruments);

        // Drain hardware MIDI events the audio callback picked up since
        // the previous iteration. Instrument delivery already happened
        // on the audio thread (within one quantum — doc #260 finding
        // #16); this pass only does the non-realtime bookkeeping:
        // record-into-clip and Thru-to-output.
        for ev in live_midi_fwd_rx.try_iter() {
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

        // Emit hardware CC/NRPN for any DeviceParam automation lane whose
        // mapped value moved since the last poll (doc #201 §4, todo #723).
        // Same engine cadence + de-duped writes as the note poll above;
        // runs for live playback and the realtime bounce drive alike, so
        // a bounced render emits the identical control stream.
        midi::poll_device_param_automation(&ctx, &mut state);

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

        // Re-apply clip edits that arrived before their clip finished
        // loading on the import worker (ba doc #276 BUG 1).
        super::clips::poll_deferred_clip_commands(&ctx, &mut state);

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
                super::retire::publish(ctx.tempo_map, Arc::new(new_tm), &ctx.shared.retired);
            }
        }

        // Free the snapshots replaced since the last tick that no reader
        // pins any more — on this thread, never the audio thread (code
        // review MIX-04).
        ctx.shared.retired.sweep();

        // Apply the take start latched by the input callback's first
        // push (doc #260 finding #2) before anything is drained against
        // the old estimate. Performer sessions subtract the measured
        // capture+playback latency so the take lands where the
        // performer heard the mix; a realtime bounce keeps the raw
        // latch — its take is aligned by the external round-trip shift
        // instead (see `bounce_realtime` + `apply_take_shift`), which
        // already covers the input side.
        if !state.rec.start_latch_applied
            && !ctx.shared.recording_start_pending.load(Ordering::Acquire)
        {
            let latched = ctx.shared.recording_start_latch.load(Ordering::Acquire);
            let io = if state.pending_bounce.is_some() {
                0
            } else {
                ctx.shared.capture_latency_samples.load(Ordering::Relaxed)
                    + ctx.shared.playback_latency_samples.load(Ordering::Relaxed)
            };
            state.rec.start_sample = latched.saturating_sub(io);
            state.rec.start_latch_applied = true;
        }

        // Drain recording ring buffer into per-track buffers
        if ctx.shared.recording.load(Ordering::Relaxed) {
            state.rec.drain_ring_to_buffers();
            // One-shot per take: report frames the capture callbacks had
            // to discard (ring overflow) so a damaged take is flagged
            // while it is still being recorded.
            state
                .rec
                .poll_overflow(&ctx.shared.recording_overflow, ctx.event_tx);
        }
        // Take-file write failures (disk full, quota): reported from the
        // drain above, a cycle-record seam or the trailing pass at stop,
        // so polled every tick rather than only while recording.
        state.rec.poll_write_errors(ctx.event_tx);

        // Print the DSP-load summary the audio thread published since
        // the last tick (it only stores atomics; formatting and stderr
        // are this thread's job — code review ARCH-02 A2-1 / ARCH-05).
        if let Some(report) = ctx.shared.cycle_report.take_new(&mut cycle_report_seen) {
            tracing::info!("{}", crate::cycle_load::format_cycle_load_line(&report));
        }
        // Same hand-off for the callback's one-shot oversize-buffer
        // warning (ARCH-05 A5-2).
        if let Some((requested, scratch)) = ctx.shared.oversize_buffer.take_unreported() {
            tracing::warn!(
                "audio: cpal requested buf={requested} frames but scratch is {scratch} — clamping; audio will run slow"
            );
        }

        super::clips::settle_clip_id_scan(&mut state, false);
        super::tracks::settle_frozen_conversions(&ctx, &mut state, false);

        // The cpal streams' error callbacks only count (FU-H6b).
        for (label, latch, limiter) in [
            (
                "output",
                &ctx.shared.output_stream_errors,
                &output_underruns,
            ),
            ("input", &ctx.shared.input_stream_errors, &input_underruns),
        ] {
            let underruns = latch.take_underruns();
            if let Some(report) = limiter.record_count(std::time::Instant::now(), underruns) {
                let line = crate::stream_errors::format_underrun_line(label, &report);
                tracing::warn!("{line}");
            }
            if let Some((count, kind)) = latch.take_errors() {
                match latch.take_error_text() {
                    Some(text) => tracing::error!(
                        "audio: {label} stream error: {kind} ({count}x); latest backend message: {text}"
                    ),
                    None => tracing::error!("audio: {label} stream error: {kind} ({count}x)"),
                }
            }
        }

        // Plugins an offline render's reset left dead (FU-M8b).
        if let Some(mut dead) = ctx.shared.plugins_dead_after_reset.try_lock() {
            for id in dead.drain(..) {
                let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::plugin(format!(
                    "Plugin instance {id} failed to restart after an offline render; it is \
                     deactivated and will stay silent."
                ))));
            }
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

        // Emit throttled (control-rate) live automated values so the app
        // can tint faders/knobs with the lane value while Read is on.
        // One batch every ~30 ms while playing; only targets whose value
        // moved are sent (doc #162 §2, todo #377). When the transport
        // isn't rolling, forget the memo once so the next play re-emits.
        if ctx.shared.playing.load(Ordering::SeqCst) {
            if last_automated_value_emit.elapsed() >= automation::AUTOMATED_VALUE_THROTTLE {
                last_automated_value_emit = std::time::Instant::now();
                let frame = ctx.shared.playhead.load(Ordering::SeqCst);
                for (target, value_norm) in
                    live_value_emitter.poll(&state.automation_lanes, frame)
                {
                    let _ = ctx.event_tx.send(AudioEvent::AutomatedValue {
                        target: target.clone(),
                        value_norm: *value_norm,
                    });
                }
            }
        } else if !live_value_emitter.is_idle() {
            live_value_emitter.reset();
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
    let drained_plugins: PluginMap =
        std::mem::take(&mut *plugins_arc.write());
    drop(drained_plugins);
}
