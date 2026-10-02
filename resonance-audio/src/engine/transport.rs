//! Transport handlers: play/record/pause/stop/seek, tempo/metronome,
//! loop range. Reads and mutates `SharedState` atomics and the tempo
//! map; `Record`/`Pause`/`Stop` also drive the recording session owned
//! by `HandlerState::rec`.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use resonance_common::{TakeContent, TimelineRange};

use crate::platform;
use crate::types::*;

use super::thread::{HandlerCtx, HandlerState, LoopRecordSession};

/// The transport side of the one "offline render in progress" gate (code
/// review MIX-02 / ENG-05): every offline renderer holds
/// `SharedState::offline_render_count` up for the length of its render,
/// on the engine thread *before* its worker spawns, so this check and
/// the renderers' own "stop the transport first" check see each other in
/// engine-thread order — there is no window in which both pass. Refuses
/// with an `AudioEvent::Error` (the app's banner) and reports `true`.
pub(crate) fn refuse_while_offline_render(ctx: &HandlerCtx, what: &str) -> bool {
    if !ctx.shared.offline_render_active() {
        return false;
    }
    let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::busy(format!(
        "Cannot {what}: {}",
        super::bounce::OFFLINE_RENDER_BUSY_MSG
    ))));
    true
}

pub(crate) fn handle_play(ctx: &HandlerCtx, state: &mut HandlerState) {
    if refuse_while_offline_render(ctx, "start playback") {
        let _ = ctx.event_tx.send(AudioEvent::TransportRefused);
        return;
    }
    let was_playing = ctx.shared.playing.load(Ordering::Relaxed);
    ctx.shared.playing.store(true, Ordering::SeqCst);
    if !was_playing {
        // Distinguish a fresh start (playhead == 0) from Continue. A
        // Start resets the receiver's song position pointer; Continue
        // resumes from wherever the playhead is right now.
        let pos = ctx.shared.playhead.load(Ordering::Relaxed);
        if pos == 0 {
            super::midi::clock_send_start(state);
        } else {
            super::midi::clock_send_continue(ctx, state, pos);
        }
        // Re-assert each external instrument's saved patch so the hardware
        // lands on the right bank/program when playback begins (doc #169).
        super::external_instrument::handle_resend_patches(ctx, state);
    }
}

/// The capture stream a record session opens: given the source device
/// (if any armed track names one), the channel count it needs and the
/// recording ring's producer, it returns the stream handle (`None` for a
/// test's fake input), the device rate and the negotiated channel count.
/// [`platform_input`] is the real one; a test passes its own so a whole
/// record run is hermetic.
pub(crate) type InputOpenResult = Result<
    (Option<crate::input_handle::InputHandle>, u32, u16),
    platform::InputStreamError,
>;

/// The real capture-stream opener: the platform backend (native PipeWire
/// or cpal), with the session's recording producer attached.
pub(crate) fn platform_input<'a>(
    ctx: &'a HandlerCtx<'a>,
) -> impl FnOnce(Option<&str>, u16, ringbuf::HeapProd<f32>) -> InputOpenResult + 'a {
    move |source_name, desired_channels, prod| {
        platform::build_input_stream(
            source_name,
            Arc::clone(ctx.shared),
            Some(prod),
            Arc::clone(ctx.monitor_prod),
            ctx.buf_frames,
            ctx.quantum,
            ctx.sample_rate,
            desired_channels,
            None,
        )
        .map(|(handle, sr, ch)| (Some(handle), sr, ch))
    }
}

/// What opening a record session came to.
pub(crate) enum SessionOpen {
    /// The input stream is up, every capturing track has its take file
    /// and the cycle-record cut is set up. Nothing is rolling or
    /// capturing yet: the caller starts the take.
    Opened,
    /// Nothing will be captured. `roll` says whether Record still starts
    /// the transport (Record degrades to Play), as it always has for
    /// every failure but a take file that will not open.
    Degraded { roll: bool },
}

pub(crate) fn handle_record(ctx: &HandlerCtx, state: &mut HandlerState, precount_bars: u8) {
    handle_record_with(ctx, state, precount_bars, platform_input(ctx));
}

/// [`handle_record`] over a given capture-stream opener (a test's fake
/// input, or [`platform_input`]).
pub(crate) fn handle_record_with(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    precount_bars: u8,
    open_input: impl FnOnce(Option<&str>, u16, ringbuf::HeapProd<f32>) -> InputOpenResult,
) {
    if refuse_while_offline_render(ctx, "start recording") {
        let _ = ctx.event_tx.send(AudioEvent::TransportRefused);
        return;
    }
    let start_sample = ctx.shared.playhead.load(Ordering::SeqCst);
    // The session — input stream, take files, cycle-record cut — opens
    // before anything rolls. A count-in then has nothing left to build
    // when it ends: the audio thread starts the take at the count-in's
    // last frame (code review RT-08), where it used to wait for this
    // thread's next tick and an input-stream rebuild of up to 500 ms
    // while the performer was already playing.
    let opened = open_recording_session(ctx, state, start_sample, open_input);
    if precount_bars == 0 {
        match opened {
            SessionOpen::Opened => start_recording_now(ctx, state),
            SessionOpen::Degraded { roll } => {
                if roll {
                    ctx.shared.playing.store(true, Ordering::SeqCst);
                }
            }
        }
        return;
    }

    // Count-in: leave the playhead exactly where the user pressed Record
    // and arm the mixer's count-in branch. The mixer holds the playhead
    // stationary and renders metronome ticks from its own elapsed counter
    // (it does not read the metronome toggle, so the toggle is left
    // alone). With a session open it also starts the take itself.
    let precount_samples = {
        let tm = ctx.tempo_map.load();
        let samples_per_bar = tm.samples_per_bar(ctx.sample_rate);
        (samples_per_bar * precount_bars as f64) as u64
    };
    let armed = matches!(opened, SessionOpen::Opened);
    if armed {
        arm_start_latch(ctx, start_sample);
        ctx.shared
            .count_in_record_arm
            .store(crate::engine::count_in_arm::ARMED, Ordering::Release);
    }
    ctx.shared
        .count_in_total
        .store(precount_samples, Ordering::SeqCst);
    ctx.shared
        .count_in_remaining
        .store(precount_samples, Ordering::SeqCst);
    ctx.shared.count_in_active.store(true, Ordering::SeqCst);
    ctx.shared.playing.store(true, Ordering::SeqCst);

    state.rec.precount = Some(crate::recording::PrecountState {
        target_sample: start_sample,
        armed,
    });
}

/// How [`settle_count_in_arm`] found the count-in → record hand-off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArmSettle {
    /// No record was waiting on a count-in.
    Idle,
    /// The take had not started; it now never will.
    Disarmed,
    /// The audio thread started the take; `recording` is set.
    Fired,
}

/// Settle the audio thread's count-in → record flip (code review RT-08)
/// so the caller sees a take that has either fully started or never
/// will. The audio thread's flip is a handful of stores between `FIRING`
/// and `FIRED`; this spins only across that window.
pub(crate) fn settle_count_in_arm(shared: &crate::engine::SharedState) -> ArmSettle {
    use crate::engine::count_in_arm::{ARMED, FIRED, IDLE};
    loop {
        match shared.count_in_record_arm.compare_exchange(
            ARMED,
            IDLE,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return ArmSettle::Disarmed,
            Err(IDLE) => return ArmSettle::Idle,
            Err(FIRED) => {
                shared.count_in_record_arm.store(IDLE, Ordering::Release);
                return ArmSettle::Fired;
            }
            Err(_) => std::hint::spin_loop(),
        }
    }
}

/// Stop / Pause, before they read `recording`: resolve a record count-in
/// still in flight. A take the audio thread already started is reported
/// started and then finalized like any other; one it had not started is
/// thrown away with its (empty) take files.
fn settle_precount(ctx: &HandlerCtx, state: &mut HandlerState) {
    let Some(pc) = state.rec.precount else {
        return;
    };
    if !pc.armed {
        return;
    }
    match settle_count_in_arm(ctx.shared) {
        ArmSettle::Fired => {
            let _ = ctx.event_tx.send(AudioEvent::RecordingStarted {
                start_sample: pc.target_sample,
            });
        }
        ArmSettle::Disarmed => abort_armed_session(ctx, state),
        ArmSettle::Idle => {}
    }
    // Armed or not, the hand-off is over; `cancel_precount` clears the
    // count-in flags.
    state.rec.precount = Some(crate::recording::PrecountState {
        armed: false,
        ..pc
    });
}

/// Tear down a session opened for a count-in that ended before its
/// downbeat: nothing was captured.
fn abort_armed_session(ctx: &HandlerCtx, state: &mut HandlerState) {
    ctx.shared
        .recording_start_pending
        .store(false, Ordering::Release);
    state.rec.abort_session();
    state.rec.input_stream = None;
    state.loop_record_session = None;
}

/// Clear any pending count-in so the mixer leaves its count-in branch.
/// Called by Pause/Stop (after [`settle_precount`]).
pub(crate) fn cancel_precount(ctx: &HandlerCtx, state: &mut HandlerState) {
    if state.rec.precount.take().is_some() {
        ctx.shared.count_in_active.store(false, Ordering::SeqCst);
        ctx.shared.count_in_remaining.store(0, Ordering::SeqCst);
        ctx.shared.count_in_total.store(0, Ordering::SeqCst);
    }
}

/// Poll hook for a count-in in flight. A record count-in's take was
/// started by the audio thread at the count-in's last frame; this only
/// does the bookkeeping once it has (`RecordingStarted`). A count-in with
/// nothing to record hands over to playback here, once the mixer has
/// drained `count_in_remaining` to zero.
pub(crate) fn poll_precount(ctx: &HandlerCtx, state: &mut HandlerState) {
    let Some(pc) = state.rec.precount else {
        return;
    };
    // The transport stopped under the count-in without a Stop / Pause
    // (those settle it themselves): drop the precount.
    if !ctx.shared.playing.load(Ordering::Relaxed) {
        settle_precount(ctx, state);
        cancel_precount(ctx, state);
        return;
    }
    if pc.armed {
        if ctx.shared.count_in_record_arm.load(Ordering::Acquire)
            != crate::engine::count_in_arm::FIRED
        {
            return;
        }
        ctx.shared
            .count_in_record_arm
            .store(crate::engine::count_in_arm::IDLE, Ordering::Release);
        state.rec.precount = None;
        ctx.shared.count_in_total.store(0, Ordering::SeqCst);
        let _ = ctx.event_tx.send(AudioEvent::RecordingStarted {
            start_sample: pc.target_sample,
        });
        return;
    }
    if ctx.shared.count_in_remaining.load(Ordering::Relaxed) > 0 {
        return;
    }
    state.rec.precount = None;
    ctx.shared.count_in_total.store(0, Ordering::SeqCst);
    ctx.shared.count_in_active.store(false, Ordering::SeqCst);
}

/// Open a record session and start it at once — the realtime bounce's
/// entry point (a plain Record goes through [`handle_record`]).
pub(crate) fn begin_recording_stream(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    start_sample: SamplePos,
) {
    match open_recording_session(ctx, state, start_sample, platform_input(ctx)) {
        SessionOpen::Opened => start_recording_now(ctx, state),
        SessionOpen::Degraded { roll } => {
            if roll {
                ctx.shared.playing.store(true, Ordering::SeqCst);
            }
        }
    }
}

/// Arm the input callbacks' start latch: the first frames pushed once
/// `recording` is set latch the raw playhead the take is placed from
/// (`SharedState::latch_recording_start`, doc #260 finding #2).
fn arm_start_latch(ctx: &HandlerCtx, start_sample: SamplePos) {
    ctx.shared
        .recording_start_latch
        .store(start_sample, Ordering::Relaxed);
    ctx.shared
        .recording_start_pending
        .store(true, Ordering::Release);
}

/// Start an opened session now: transport and capture together.
fn start_recording_now(ctx: &HandlerCtx, state: &mut HandlerState) {
    // Transport + capture start together, *after* the stream is up: the
    // first pushed frames latch the aligned take start, which the engine
    // loop applies before the first drain (doc #260 finding #2).
    arm_start_latch(ctx, state.rec.start_sample);
    ctx.shared.playing.store(true, Ordering::SeqCst);
    ctx.shared.recording.store(true, Ordering::SeqCst);
    let _ = ctx.event_tx.send(AudioEvent::RecordingStarted {
        start_sample: state.rec.start_sample,
    });
}

/// Open a record session without starting it: the input stream with the
/// recording producer attached, a take file per capturing armed track,
/// the cycle-record session and cut. `playing` / `recording` are left to
/// the caller — a plain Record starts at once, a count-in leaves it to
/// the audio thread.
pub(crate) fn open_recording_session(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    start_sample: SamplePos,
    open_input: impl FnOnce(Option<&str>, u16, ringbuf::HeapProd<f32>) -> InputOpenResult,
) -> SessionOpen {
    // Fresh session: no take shift unless the realtime bounce sets one
    // after this returns (external-instrument round-trip compensation).
    state.rec.take_shift_samples = 0;
    // Recording must have a project directory to stream WAVs into.
    // The startup modal guarantees a project is always selected, so
    // hitting this branch is a programmer error — surface it rather
    // than silently losing the take.
    let project_dir = match state.project_dir.clone() {
        Some(dir) => dir,
        None => {
            let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::internal(
                "Cannot record: no project directory set. Open or create a project first.",
            )));
            return SessionOpen::Degraded { roll: false };
        }
    };

    // NOTE: `playing` is deliberately NOT set here. It used to flip
    // before the input stream was even built (an up-to-500 ms open), so
    // the playhead ran ahead while no frames could be captured and that
    // variable gap landed inside every take (doc #260 finding #2). The
    // caller starts the transport once the session is open.

    // Snapshot port + mono per armed track so the drain loop on the
    // engine thread doesn't need to re-lock the tracks map for every
    // buffer pop.
    struct ArmedInfo {
        track_id: TrackId,
        device: Option<String>,
        port: u16,
        mono: bool,
        /// Whether this track's signal arrives as audio on the input, and
        /// so needs a recording buffer + WAV writer — see
        /// [`Track::runs_internal_instrument`]. An armed track whose
        /// instrument runs in-process still counts as armed (it records a
        /// MIDI performance) but captures no audio.
        captures_audio: bool,
    }
    let armed_tracks: Vec<ArmedInfo> = {
        let tracks_guard = ctx.tracks();
        tracks_guard
            .values()
            .filter(|t| t.record_armed())
            .map(|t| ArmedInfo {
                track_id: t.id,
                device: t.input_device_name.load_full().map(|a| (*a).clone()),
                port: t.input_port(),
                mono: t.mono(),
                captures_audio: !t.runs_internal_instrument(),
            })
            .collect()
    };

    if armed_tracks.is_empty() {
        // Nothing to record: Record degrades to Play, as before.
        return SessionOpen::Degraded { roll: true };
    }

    // Every capturing track needs a clip id from the app's grant (ARCH-04
    // D-7d, design doc D-6 §4.2 C3). Checked before the input stream
    // opens, and all or nothing: no half-armed take. Out of ids is the
    // "Failed to start recording" branch below — the transport still rolls
    // and nothing is captured (§7a.3) — with a `Busy` error the app shows
    // as a banner. Reachable only when the app has stopped answering
    // `IdGrantLow` (or never granted anything).
    let capturing = armed_tracks.iter().filter(|i| i.captures_audio).count() as u64;
    if state.clip_grant.len() < capturing {
        state.report_clip_grant_low(ctx.event_tx);
        let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::busy(
            "Failed to start recording: no clip ids available — try again",
        )));
        return SessionOpen::Degraded { roll: true };
    }

    let source_name: Option<String> = armed_tracks.iter().find_map(|info| info.device.clone());

    // Highest input channel any armed track needs. Required so
    // cpal/PipeWire opens the capture node with enough channels for
    // tracks that pick port 2+ — without this, the stream is stereo
    // and the deinterleave clamps to channel 1.
    let desired_channels: u16 = armed_tracks
        .iter()
        .map(|info| if info.mono { info.port + 1 } else { info.port + 2 })
        .max()
        .unwrap_or(2)
        .max(2);

    // Drop any existing input stream first so the backend (PipeWire)
    // can release the source before the new connection opens —
    // otherwise the second open might race the teardown of the old
    // monitor stream and end up with the old channel count.
    state.rec.input_stream = None;

    // Sized in frames of the stream's width, not a fixed sample count
    // (code review RT-17). The stream can negotiate more channels than
    // asked for (the cpal fallback opens the device default); the ring
    // then holds proportionally less time, still whole frames.
    let ring = ringbuf::HeapRb::<f32>::new(super::recording_ring_len(
        ctx.sample_rate,
        desired_channels,
    ));
    use ringbuf::traits::Split;
    let (prod, cons) = ring.split();

    // Build the input stream first so we know the device's actual
    // sample rate; the streaming resamplers need it at track-buf
    // creation time.
    let (stream, in_sr, in_ch) = match open_input(source_name.as_deref(), desired_channels, prod) {
        Ok(triple) => triple,
        Err(e) => {
            // Keep the legacy behaviour: the transport rolls even when
            // the recording stream could not be opened.
            let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::new(
                e.kind(),
                format!("Failed to start recording: {}", e),
            )));
            return SessionOpen::Degraded { roll: true };
        }
    };

    // Open a cycle-record session when loop-record mode is armed and a
    // real loop range is active. Each pass then becomes a distinct take,
    // its audio cut by sample count (`poll_recording_capture`, code
    // review RT-02) and its MIDI at the playhead's wrap
    // (`poll_loop_record_seam`); without it a looped recording keeps the
    // legacy single-clip behaviour.
    let cycle_slot = (state.rec.loop_record
        && state.rec.loop_enabled
        && state.rec.loop_out > state.rec.loop_in)
        .then_some((state.rec.loop_in, state.rec.loop_out));

    // The audio cut only matters when there is audio to cut.
    state.rec.begin_session(start_sample, cycle_slot.filter(|_| capturing > 0));
    // The PDC the performer will hear the mix behind, latched now so a
    // latency change mid-take cannot move the take (code review RT-01).
    state.rec.record_pdc_samples = ctx.latency_comp.load().max_latency()
        + ctx.shared.master_latency_samples.load(Ordering::Relaxed);
    // A new record take starts with a clean dropped-frame count and a
    // re-armed `RecordingOverflow` report.
    state
        .rec
        .begin_overflow_episode(&ctx.shared.recording_overflow);
    state.rec.ring_consumer = Some(cons);
    state.rec.input_sample_rate = in_sr;
    state.rec.input_channels = in_ch;

    // Allocate a clip id per armed track that actually captures audio and
    // open a WAV writer targeting its final location in the project's
    // audio dir. Any failure here unwinds the partially-built state and
    // bails.
    //
    // Tracks whose instrument runs in-process are skipped: they are armed
    // to record a MIDI performance, and giving them a buffer would write a
    // WAV of whatever happened to be on the input and — under cycle-record
    // — file a second, spurious take for every loop pass (ba doc #292).
    // They stay in `armed_tracks` so the transport still enters recording
    // and opens the cycle-record session for their MIDI.
    //
    // Ids come from the app's grant, in order (D-7d). One whose WAV already
    // exists is skipped, never overwritten (`ClipIdGrant::take_unused_wav`),
    // which can leave the grant short despite the check above; that is the
    // same unwind as a file that fails to open.
    let audio_dir = project_dir.join("audio");
    for info in armed_tracks.iter().filter(|i| i.captures_audio) {
        let drawn = state.clip_grant.take_unused_wav(&audio_dir);
        state.report_clip_grant_low(ctx.event_tx);
        let Some(clip_id) = drawn else {
            state.rec.abort_session();
            let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::busy(
                "Failed to start recording: no clip ids available — try again",
            )));
            return SessionOpen::Degraded { roll: true };
        };
        match crate::recording::RecordingState::create_track_buf(
            &project_dir,
            info.track_id,
            clip_id,
            ctx.sample_rate,
            in_sr,
            info.port,
            info.mono,
        ) {
            Ok(buf) => {
                state.rec.buffers.insert(info.track_id, buf);
            }
            Err(e) => {
                state.rec.abort_session();
                let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::io(format!(
                    "Failed to open recording file: {e}"
                ))));
                return SessionOpen::Degraded { roll: false };
            }
        }
    }

    state.rec.input_stream = stream;
    ctx.shared.input_channels.store(in_ch, Ordering::Release);

    state.loop_record_session = cycle_slot.map(|(loop_in, loop_out)| LoopRecordSession {
        slot: TimelineRange::from_bounds(loop_in, loop_out),
        pass_index: 0,
    });
    SessionOpen::Opened
}

/// Close the take a Stop / Pause ends: its placement latch, the trailing
/// cycle-record pass(es) or the single trimmed clip, then the stream.
fn finish_recording_session(ctx: &HandlerCtx, state: &mut HandlerState) {
    // A take stopped within a tick of its first push has not had its
    // latch applied by the engine loop yet.
    apply_recording_start_latch(ctx, state);
    // Last chance to report an overflow the periodic drain poll has
    // not seen yet (latched — a no-op when it already fired).
    state
        .rec
        .poll_overflow(&ctx.shared.recording_overflow, ctx.event_tx);
    if state.loop_record_session.is_some() {
        // Cycle-record: emit the trailing pass as its own take instead
        // of the legacy single trimmed clip.
        finalize_loop_record_pass(ctx, state, false);
    } else {
        // The takes join the render graph's clip list in one publish.
        let rec = &mut state.rec;
        ctx.shared
            .edit_clips(|clips| rec.finalize_recording(ctx.sample_rate, clips, ctx.event_tx));
    }
    state.rec.input_stream = None;
}

pub(crate) fn handle_pause(ctx: &HandlerCtx, state: &mut HandlerState) {
    // Before `recording` is read: a record count-in's take has either
    // been started by the audio thread or never will be (RT-08).
    settle_precount(ctx, state);
    let was_recording = ctx.shared.recording.load(Ordering::SeqCst);
    let was_playing = ctx.shared.playing.load(Ordering::Relaxed);
    ctx.shared.playing.store(false, Ordering::SeqCst);
    ctx.shared.recording.store(false, Ordering::SeqCst);
    cancel_precount(ctx, state);

    if was_recording {
        finish_recording_session(ctx, state);
    }
    panic_all_instrument_plugins(ctx);
    let pause_sample = ctx.shared.playhead.load(Ordering::SeqCst);
    super::midi::close_open_recordings(ctx, state, pause_sample);
    // A new run reports its own out-of-ids error (D-7d, C5).
    state.live_midi_no_id_reported = false;
    // close_open_recordings bails when no recording is active, so call
    // all-notes-off directly to silence hardware synths driven by the
    // timeline.
    state.midi_hw.midi_outputs.all_notes_off_everywhere();
    if was_playing {
        super::midi::clock_send_stop(state);
    }
}

pub(crate) fn handle_stop(ctx: &HandlerCtx, state: &mut HandlerState) {
    // Before `recording` is read: a record count-in's take has either
    // been started by the audio thread or never will be (RT-08).
    settle_precount(ctx, state);
    let was_recording = ctx.shared.recording.load(Ordering::SeqCst);
    let was_playing = ctx.shared.playing.load(Ordering::Relaxed);
    // Where the transport stopped: held recorded notes close here, not
    // at the rewound playhead (code review FU-A2c).
    let stop_sample = ctx.shared.playhead.load(Ordering::SeqCst);
    ctx.shared.playing.store(false, Ordering::SeqCst);
    ctx.shared.recording.store(false, Ordering::SeqCst);
    ctx.shared.playhead.store(0, Ordering::SeqCst);
    cancel_precount(ctx, state);

    if was_recording {
        finish_recording_session(ctx, state);
    }

    panic_all_instrument_plugins(ctx);
    super::midi::close_open_recordings(ctx, state, stop_sample);
    // A new run reports its own out-of-ids error (D-7d, C5).
    state.live_midi_no_id_reported = false;
    state.midi_hw.midi_outputs.all_notes_off_everywhere();
    if was_playing {
        super::midi::clock_send_stop(state);
    }
    // Park the master clock at song start so the next Play emits a
    // fresh Start (or Continue from 0) rather than resuming from the
    // end of the prior segment.
    super::midi::clock_send_song_position(ctx, state, 0);

    let _ = ctx.event_tx.send(AudioEvent::Stopped);
}

/// Send all-notes-off to every instrument plugin's primary instance.
/// Called from Pause, Stop and Seek so a hardware key still held when the
/// user pauses doesn't leave the plugin sustaining indefinitely (no
/// hardware NoteOff will arrive once the user lets go past Pause).
///
/// `all_notes_off` only *queues* 128 NoteOff events into the plugin's
/// pending buffer; they're drained on the next `process()` call. When
/// the audio mixer is in its stopped branch with no monitor track
/// active it never calls `process()` on these plugins, so we drive a
/// one-block silent process pass right here. We deliberately use
/// `try_lock` rather than blocking — the audio thread's own try_lock
/// would otherwise fail for whatever block straddles this call and
/// silence the plugin's tail. An instrument the audio thread is holding
/// is skipped here; the audio thread flushes it itself, because it
/// notices the playhead jump (or the stop) and issues its own
/// all-notes-off, parked in its MIDI stash on contention rather than
/// lost (code review MIX-06).
fn panic_all_instrument_plugins(ctx: &HandlerCtx) {
    let tracks_guard = ctx.tracks();
    let plugins_guard = ctx.plugins();
    let mut silent_l = [0.0f32; 64];
    let mut silent_r = [0.0f32; 64];
    for track in tracks_guard.values() {
        if track.track_type.accepts_midi() {
            if let Some(&inst_id) = track.plugins().first() {
                if let Some(mutex) = plugins_guard.get(&inst_id) {
                    if let Some(mut inst) = mutex.try_lock() {
                        inst.0.all_notes_off_and_drop_carried();
                        inst.0.process(&mut silent_l, &mut silent_r, 64);
                    }
                }
            }
        }
    }
}

pub(crate) fn handle_seek_to(ctx: &HandlerCtx, state: &mut HandlerState, pos: u64) {
    // Flush sounding notes before the jump, same as Stop and the loop
    // seam — notes started before the seek would otherwise never see
    // their NoteOff and sustain forever.
    if ctx.shared.playing.load(Ordering::Relaxed) {
        panic_all_instrument_plugins(ctx);
        state.midi_hw.midi_outputs.all_notes_off_everywhere();
    }
    ctx.shared.playhead.store(pos, Ordering::SeqCst);
    super::midi::clock_send_song_position(ctx, state, pos);
}

/// Set the flat tempo. Goes through the one tempo-legality rule
/// (`sanitize_bpm`, FU-D3): out-of-range values clamp into
/// `MIN_BPM..=MAX_BPM`, and a non-finite one is ignored — a NaN tempo
/// would turn every bar position into NaN (→ sample 0).
pub(crate) fn handle_set_bpm(ctx: &HandlerCtx, bpm: f32) {
    let Some(bpm) = crate::types::sanitize_bpm(bpm) else {
        return;
    };
    super::rcu_tempo(ctx, |tm| tm.bpm = bpm);
}

pub(crate) fn handle_set_tempo_events(
    ctx: &HandlerCtx,
    tempo: Vec<crate::types::TempoPoint>,
    signature: Vec<crate::types::SignaturePoint>,
) {
    let sample_rate = ctx.sample_rate;
    super::rcu_tempo(ctx, |tm| {
        if let Some(first) = tempo.first() {
            tm.bpm = first.bpm;
        }
        tm.tempo_points = tempo;
        tm.signature_points = signature;
        tm.rebuild_bar_table(sample_rate);
    });
}

pub(crate) fn handle_set_time_signature(ctx: &HandlerCtx, numerator: u8, denominator: u8) {
    super::rcu_tempo(ctx, |tm| {
        tm.numerator = numerator.max(1);
        tm.denominator = denominator.max(1);
    });
}

pub(crate) fn handle_set_metronome_enabled(ctx: &HandlerCtx, enabled: bool) {
    super::rcu_tempo(ctx, |tm| tm.metronome_enabled = enabled);
}

pub(crate) fn handle_set_loop_range(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    enabled: bool,
    loop_in: u64,
    loop_out: u64,
) {
    state.rec.loop_enabled = enabled;
    state.rec.loop_in = loop_in;
    state.rec.loop_out = loop_out;
    // One publish for the whole range (code review RT-13).
    ctx.shared
        .set_loop_range(crate::engine::LoopRange::new(enabled, loop_in, loop_out));
}

/// Toggle cycle-record (loop-record) mode. Stored on the recording state
/// so [`begin_recording_stream`] opens a [`LoopRecordSession`] when the
/// next record starts inside an active loop range.
pub(crate) fn handle_set_loop_record_mode(state: &mut HandlerState, on: bool) {
    state.rec.loop_record = on;
}

/// Place the take from the start the input callback latched at its first
/// push (doc #260 finding #2), once that push has happened. A performer
/// take lands where the performer heard the mix: the latch minus the
/// measured capture + playback latency and the PDC latched when the
/// session opened (code review RT-01). A realtime bounce keeps the raw
/// latch — its take is aligned by the external round-trip shift instead
/// (see `bounce_realtime` + `apply_take_shift`), which already covers the
/// input side.
pub(crate) fn apply_recording_start_latch(ctx: &HandlerCtx, state: &mut HandlerState) {
    if state.rec.start_latch_applied
        || ctx.shared.recording_start_pending.load(Ordering::Acquire)
    {
        return;
    }
    let latched = ctx.shared.recording_start_latch.load(Ordering::Acquire);
    let compensation = if state.pending_bounce.is_some() {
        0
    } else {
        ctx.shared.capture_latency_samples.load(Ordering::Relaxed)
            + ctx.shared.playback_latency_samples.load(Ordering::Relaxed)
            + state.rec.record_pdc_samples
    };
    state.rec.apply_start_latch(latched, compensation);
}

/// The engine loop's recording step: apply the start latch, stream the
/// captured input into the take files, and roll every cycle-record pass
/// that is complete — at its exact cut, counted in input frames (code
/// review RT-02), however late this tick runs. The frames past a cut go
/// to the next pass's writer.
pub(crate) fn poll_recording_capture(ctx: &HandlerCtx, state: &mut HandlerState) {
    apply_recording_start_latch(ctx, state);
    if !ctx.shared.recording.load(Ordering::Relaxed) {
        return;
    }
    loop {
        state.rec.drain_ring_to_buffers();
        if state.loop_record_session.is_none() || !state.rec.pass_complete() {
            break;
        }
        roll_loop_record_audio_pass(ctx, state, true);
    }
    // One-shot per take: report frames the capture callbacks had to
    // discard (ring overflow) so a damaged take is flagged while it is
    // still being recorded.
    state
        .rec
        .poll_overflow(&ctx.shared.recording_overflow, ctx.event_tx);
}

/// Detect a loop wrap during a cycle-record run and roll the finished
/// MIDI pass.
///
/// Runs on the engine control thread every iteration. `last_playhead`
/// carries the previous-iteration position across calls; when the playhead
/// has moved backwards (the audio thread wrapped `loop_out` → `loop_in`)
/// while recording with an open [`LoopRecordSession`], the just-completed
/// pass's notes become one take per armed instrument track. Audio passes
/// are not cut here: they are cut by sample count in
/// [`poll_recording_capture`] (code review RT-02).
pub(crate) fn poll_loop_record_seam(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    last_playhead: &mut SamplePos,
) {
    let playhead = ctx.shared.playhead.load(Ordering::Relaxed);
    let wrapped = playhead < *last_playhead;
    *last_playhead = playhead;
    if !wrapped
        || !ctx.shared.recording.load(Ordering::Relaxed)
        || state.loop_record_session.is_none()
    {
        return;
    }
    roll_loop_record_midi_pass(ctx, state, true);
}

/// Finalize a cycle-record run at transport stop: every audio pass still
/// in the ring at its exact cut, then the trailing audio and MIDI passes,
/// and tear the session down. (`reopen = true` rolls one seam of both,
/// for a caller that drives the seam by hand.)
pub(crate) fn finalize_loop_record_pass(ctx: &HandlerCtx, state: &mut HandlerState, reopen: bool) {
    if state.project_dir.is_none() || state.loop_record_session.is_none() {
        return;
    }
    if reopen {
        roll_loop_record_audio_pass(ctx, state, true);
        roll_loop_record_midi_pass(ctx, state, true);
        return;
    }
    // Complete passes first, each cut where its frames end; the frames the
    // input pushed before it stopped then make the trailing pass.
    loop {
        state.rec.drain_ring_to_buffers();
        if !(state.rec.pass_complete() && state.rec.has_pending_input()) {
            break;
        }
        roll_loop_record_audio_pass(ctx, state, true);
    }
    roll_loop_record_audio_pass(ctx, state, false);
    roll_loop_record_midi_pass(ctx, state, false);
    state.loop_record_session = None;
}

/// Roll the current cycle-record audio pass into one take per capturing
/// track and emit `AudioEvent::TakeCaptured` for each. With `reopen` a
/// fresh writer takes the next pass.
///
/// The punch-in pass is placed where the take started — the latched,
/// compensated start (RT-01); every later pass at the loop start, since
/// its cut already sits where the performer heard the loop wrap (RT-02).
fn roll_loop_record_audio_pass(ctx: &HandlerCtx, state: &mut HandlerState, reopen: bool) {
    let Some(audio_dir) = state.project_dir.as_ref().map(|d| d.join("audio")) else {
        return;
    };
    let Some(slot) = state.loop_record_session.as_ref().map(|s| s.slot) else {
        return;
    };
    let clip_start = if state.rec.audio_passes_rolled == 0 {
        state.rec.start_sample
    } else {
        slot.start
    };

    // Report any ring overflow that damaged the pass being finalized
    // (latched — a no-op when the drain poll already fired), then start
    // the next pass with a clean count: a new take starts clean.
    state
        .rec
        .poll_overflow(&ctx.shared.recording_overflow, ctx.event_tx);
    if reopen {
        state
            .rec
            .begin_overflow_episode(&ctx.shared.recording_overflow);
    }

    // The pass's takes join the render graph's clip list in one publish.
    // The next pass's clip ids come from the app's grant (ARCH-04 D-7d);
    // a track that finds it empty keeps this pass and records no further
    // ones (`roll_audio_pass`'s reopen-failure branch, design doc D-6 §4.2
    // C4).
    let (rec, grant) = (&mut state.rec, &mut state.clip_grant);
    let rolled = ctx.shared.edit_clips(|clips| {
        rec.roll_audio_pass(ctx.sample_rate, clip_start, clips, &audio_dir, grant, reopen)
    });
    state.report_clip_grant_low(ctx.event_tx);
    let mut captured_any = false;
    for take in rolled {
        let content = TakeContent::Audio {
            clip_ref: take.clip_id,
        };
        // What this pass really recorded over — the punch-in point for
        // pass 0, and short of the loop end for a pass cut off at stop.
        // Reported so the app's promote clamp and take lane stop assuming
        // every take fills its slot (ba todo #1396). It is the run's own
        // measurement even when the take joins an existing lane.
        let extent = take.extent();
        let _ = ctx
            .event_tx
            .send(capture_take(state, take.track_id, slot, extent, content));
        captured_any = true;
    }
    // Republish the comp playback table so the pass just captured is
    // immediately audible — with no comp and no active take yet, the
    // default cover is the most recent take.
    if captured_any {
        super::takes::publish_take_comp(ctx, state);
    }
}

/// Roll the current cycle-record MIDI pass (instrument tracks) into one
/// take per track and emit `AudioEvent::TakeCaptured` for each.
fn roll_loop_record_midi_pass(ctx: &HandlerCtx, state: &mut HandlerState, reopen: bool) {
    let Some(slot) = state.loop_record_session.as_ref().map(|s| s.slot) else {
        return;
    };
    let midi_takes = super::midi::capture_loop_record_midi_pass(ctx, state, slot.end());
    let mut captured_any = false;
    for (track_id, notes) in midi_takes {
        let content = TakeContent::Midi { notes };
        // A MIDI take's extent is the whole region the run cycled over:
        // its notes are its content, silence inside it is a rest rather
        // than a hole, and there is no medium that can come up short (ba
        // todo #1396, and `Take::audible_extent`).
        let _ = ctx
            .event_tx
            .send(capture_take(state, track_id, slot, slot, content));
        captured_any = true;
    }
    if captured_any {
        super::takes::publish_take_comp(ctx, state);
    }
    if reopen {
        if let Some(s) = state.loop_record_session.as_mut() {
            s.pass_index += 1;
        }
    }
}


/// File one captured take against the lane it belongs to and return the
/// `TakeCaptured` event to send. A thin `HandlerState` adapter over
/// `takes::capture_take_event`, which is where the whole rule lives:
/// the lookup is against `state.take_groups`, which outlives the record run
/// and holds groups restored from a saved project, so a second, third or
/// post-reload run over the same region folds into the lane already there
/// (todo #1392).
fn capture_take(
    state: &mut HandlerState,
    track_id: TrackId,
    run_slot: TimelineRange,
    extent: TimelineRange,
    content: TakeContent,
) -> AudioEvent {
    // Disjoint field borrows: the store is written, the allocator is bumped.
    super::takes::capture_take_event(
        &mut state.take_groups,
        &mut state.next_take_group_id,
        track_id,
        run_slot,
        extent,
        content,
    )
}
