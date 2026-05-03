//! Transport handlers: play/record/pause/stop/seek, tempo/metronome,
//! loop range. Reads and mutates `SharedState` atomics and the tempo
//! map; `Record`/`Pause`/`Stop` also drive the recording session owned
//! by `HandlerState::rec`.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::platform;
use crate::types::*;

use super::thread::{HandlerCtx, HandlerState};

pub(crate) fn handle_play(ctx: &HandlerCtx) {
    ctx.shared.playing.store(true, Ordering::SeqCst);
}

pub(crate) fn handle_record(ctx: &HandlerCtx, state: &mut HandlerState, precount_bars: u8) {
    if precount_bars == 0 {
        let start_sample = ctx.shared.playhead.load(Ordering::SeqCst);
        begin_recording_stream(ctx, state, start_sample);
        return;
    }

    // Count-in: leave the playhead exactly where the user pressed
    // Record and arm the mixer's count-in branch. The mixer holds
    // the playhead stationary, renders metronome ticks from its own
    // elapsed counter, and the engine control thread opens the
    // recording stream the moment `count_in_remaining` reaches zero.
    let (precount_samples, was_metronome) = {
        let tm = ctx.tempo_map.read();
        let samples_per_bar = tm.samples_per_bar(ctx.sample_rate);
        let samples = (samples_per_bar * precount_bars as f64) as u64;
        (samples, tm.metronome_enabled)
    };

    let orig_playhead = ctx.shared.playhead.load(Ordering::SeqCst);
    ctx.shared
        .count_in_total
        .store(precount_samples, Ordering::SeqCst);
    ctx.shared
        .count_in_remaining
        .store(precount_samples, Ordering::SeqCst);
    ctx.shared.count_in_active.store(true, Ordering::SeqCst);
    ctx.tempo_map.write().metronome_enabled = true;
    ctx.shared.playing.store(true, Ordering::SeqCst);

    state.rec.precount = Some(crate::recording::PrecountState {
        target_sample: orig_playhead,
        restore_metronome: was_metronome,
    });
}

/// Clear any pending count-in and restore the metronome toggle to
/// whatever it was before `Record` was pressed. Called by Pause/Stop
/// so cancelling a record-with-precount doesn't leave the metronome
/// stuck on or the mixer stuck in its count-in branch.
pub(crate) fn cancel_precount(ctx: &HandlerCtx, state: &mut HandlerState) {
    if let Some(pc) = state.rec.precount.take() {
        ctx.tempo_map.write().metronome_enabled = pc.restore_metronome;
        ctx.shared.count_in_active.store(false, Ordering::SeqCst);
        ctx.shared.count_in_remaining.store(0, Ordering::SeqCst);
        ctx.shared.count_in_total.store(0, Ordering::SeqCst);
    }
}

/// Poll hook: if a count-in is in flight and the mixer has drained
/// `count_in_remaining` to zero, restore the metronome toggle and
/// open the actual recording stream. Runs on the engine control
/// thread's ~60 Hz loop, so the worst-case start jitter is one
/// engine tick (≈16 ms) on top of the one-buffer tail the mixer
/// holds after the counter hits zero.
pub(crate) fn poll_precount(ctx: &HandlerCtx, state: &mut HandlerState) {
    let Some(pc) = state.rec.precount else {
        return;
    };
    // Pause/Stop clears `playing` — if that happened while counting
    // in, drop the precount without starting the stream.
    if !ctx.shared.playing.load(Ordering::Relaxed) {
        state.rec.precount = None;
        ctx.tempo_map.write().metronome_enabled = pc.restore_metronome;
        ctx.shared.count_in_active.store(false, Ordering::SeqCst);
        ctx.shared.count_in_remaining.store(0, Ordering::SeqCst);
        ctx.shared.count_in_total.store(0, Ordering::SeqCst);
        return;
    }
    if ctx.shared.count_in_remaining.load(Ordering::Relaxed) > 0 {
        return;
    }
    state.rec.precount = None;
    ctx.tempo_map.write().metronome_enabled = pc.restore_metronome;
    ctx.shared.count_in_total.store(0, Ordering::SeqCst);
    // The mixer held the playhead stationary through the count-in,
    // so it still points at the punch-in line. Open the recording
    // stream first, then clear `count_in_active` — the mixer keeps
    // holding the playhead until this flag flips, which is what
    // makes the count-in → record transition race-free even if
    // CPAL's `build_input_stream` takes real wall-clock time.
    begin_recording_stream(ctx, state, pc.target_sample);
    ctx.shared.count_in_active.store(false, Ordering::SeqCst);
}

fn begin_recording_stream(ctx: &HandlerCtx, state: &mut HandlerState, start_sample: SamplePos) {
    // Recording must have a project directory to stream WAVs into.
    // The startup modal guarantees a project is always selected, so
    // hitting this branch is a programmer error — surface it rather
    // than silently losing the take.
    let project_dir = match state.project_dir.clone() {
        Some(dir) => dir,
        None => {
            let _ = ctx.event_tx.send(AudioEvent::Error(
                "Cannot record: no project directory set. Open or create a project first.".into(),
            ));
            return;
        }
    };

    ctx.shared.playing.store(true, Ordering::SeqCst);

    // Snapshot port + mono per armed track so the drain loop on the
    // engine thread doesn't need to re-lock the tracks map for every
    // buffer pop.
    struct ArmedInfo {
        track_id: TrackId,
        device: Option<String>,
        port: u16,
        mono: bool,
    }
    let armed_tracks: Vec<ArmedInfo> = {
        let tracks_guard = ctx.tracks.read();
        tracks_guard
            .values()
            .filter(|t| t.record_armed())
            .map(|t| ArmedInfo {
                track_id: t.id,
                device: t.input_device_name.clone(),
                port: t.input_port(),
                mono: t.mono(),
            })
            .collect()
    };

    if armed_tracks.is_empty() {
        return;
    }

    let source_name: Option<String> = armed_tracks.iter().find_map(|info| info.device.clone());

    let ring_size = super::RECORDING_RING_SIZE;
    let ring = ringbuf::HeapRb::<f32>::new(ring_size);
    use ringbuf::traits::Split;
    let (prod, cons) = ring.split();

    // Build the input stream first so we know the device's actual
    // sample rate; the streaming resamplers need it at track-buf
    // creation time.
    let (stream, in_sr, in_ch) = match platform::build_input_stream(
        source_name.as_deref(),
        Arc::clone(ctx.shared),
        Some(prod),
        Arc::clone(ctx.monitor_prod),
        ctx.buf_frames,
        ctx.quantum,
    ) {
        Ok(triple) => triple,
        Err(e) => {
            let _ = ctx.event_tx.send(AudioEvent::Error(format!(
                "Failed to start recording: {}",
                e
            )));
            return;
        }
    };

    state.rec.start_sample = start_sample;
    state.rec.ring_consumer = Some(cons);
    state.rec.input_sample_rate = in_sr;
    state.rec.input_channels = in_ch;

    // Allocate a clip id per armed track and open a WAV writer
    // targeting its final location in the project's audio dir. Any
    // failure here unwinds the partially-built state and bails.
    for info in &armed_tracks {
        let clip_id = state.next_clip_id;
        state.next_clip_id += 1;
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
                state.rec.buffers.clear();
                state.rec.ring_consumer = None;
                let _ = ctx.event_tx.send(AudioEvent::Error(format!(
                    "Failed to open recording file: {e}"
                )));
                return;
            }
        }
    }

    state.rec.input_stream = Some(stream);
    ctx.shared.input_channels.store(in_ch, Ordering::Release);
    ctx.shared.recording.store(true, Ordering::SeqCst);

    let _ = ctx.event_tx.send(AudioEvent::RecordingStarted {
        start_sample: state.rec.start_sample,
    });
}

pub(crate) fn handle_pause(ctx: &HandlerCtx, state: &mut HandlerState) {
    let was_recording = ctx.shared.recording.load(Ordering::SeqCst);
    ctx.shared.playing.store(false, Ordering::SeqCst);
    ctx.shared.recording.store(false, Ordering::SeqCst);
    cancel_precount(ctx, state);

    if was_recording {
        state
            .rec
            .finalize_recording(ctx.sample_rate, ctx.clips.as_ref(), ctx.event_tx);
        state.rec.input_stream = None;
    }
    super::midi::close_open_recordings(ctx, state);
}

pub(crate) fn handle_stop(ctx: &HandlerCtx, state: &mut HandlerState) {
    let was_recording = ctx.shared.recording.load(Ordering::SeqCst);
    ctx.shared.playing.store(false, Ordering::SeqCst);
    ctx.shared.recording.store(false, Ordering::SeqCst);
    ctx.shared.playhead.store(0, Ordering::SeqCst);
    cancel_precount(ctx, state);

    if was_recording {
        state
            .rec
            .finalize_recording(ctx.sample_rate, ctx.clips.as_ref(), ctx.event_tx);
        state.rec.input_stream = None;
    }

    // Send all-notes-off to instrument plugins to prevent stuck notes
    {
        let tracks_guard = ctx.tracks.read();
        let plugins_guard = ctx.plugins.read();
        for track in tracks_guard.values() {
            if track.track_type == TrackType::Instrument {
                if let Some(&inst_id) = track.plugin_ids.first() {
                    if let Some(mutex) = plugins_guard.get(&inst_id) {
                        let mut inst = mutex.lock();
                        inst.0.all_notes_off();
                    }
                }
            }
        }
    }
    super::midi::close_open_recordings(ctx, state);

    let _ = ctx.event_tx.send(AudioEvent::Stopped);
}

pub(crate) fn handle_seek_to(ctx: &HandlerCtx, pos: u64) {
    ctx.shared.playhead.store(pos, Ordering::SeqCst);
}

pub(crate) fn handle_set_bpm(ctx: &HandlerCtx, bpm: f32) {
    ctx.tempo_map.write().bpm = bpm.clamp(20.0, 999.0);
}

pub(crate) fn handle_set_tempo_events(
    ctx: &HandlerCtx,
    tempo: Vec<crate::types::TempoPoint>,
    signature: Vec<crate::types::SignaturePoint>,
) {
    let mut tm = ctx.tempo_map.write();
    if let Some(first) = tempo.first() {
        tm.bpm = first.bpm;
    }
    tm.tempo_points = tempo;
    tm.signature_points = signature;
    tm.rebuild_bar_table(ctx.sample_rate);
}

pub(crate) fn handle_set_time_signature(ctx: &HandlerCtx, numerator: u8, denominator: u8) {
    let mut tm = ctx.tempo_map.write();
    tm.numerator = numerator.max(1);
    tm.denominator = denominator.max(1);
}

pub(crate) fn handle_set_metronome_enabled(ctx: &HandlerCtx, enabled: bool) {
    ctx.tempo_map.write().metronome_enabled = enabled;
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
    ctx.shared.loop_enabled.store(enabled, Ordering::Relaxed);
    ctx.shared.loop_in.store(loop_in, Ordering::Relaxed);
    ctx.shared.loop_out.store(loop_out, Ordering::Relaxed);
}
