//! Transport handlers: play/record/pause/stop/seek, tempo/metronome,
//! loop range. Reads and mutates `SharedState` atomics and the tempo
//! map; `Record`/`Pause`/`Stop` also drive the recording session owned
//! by `HandlerState::rec`.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use resonance_common::{TakeContent, TakeGroupId, TimelineRange};

use crate::platform;
use crate::types::*;

use super::thread::{HandlerCtx, HandlerState, LoopRecordSession};

pub(crate) fn handle_play(ctx: &HandlerCtx, state: &mut HandlerState) {
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
    }
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
        let tm = ctx.tempo_map.load();
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
    super::rcu_tempo(ctx.tempo_map, |tm| tm.metronome_enabled = true);
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
        super::rcu_tempo(ctx.tempo_map, |tm| {
            tm.metronome_enabled = pc.restore_metronome
        });
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
        super::rcu_tempo(ctx.tempo_map, |tm| {
            tm.metronome_enabled = pc.restore_metronome
        });
        ctx.shared.count_in_active.store(false, Ordering::SeqCst);
        ctx.shared.count_in_remaining.store(0, Ordering::SeqCst);
        ctx.shared.count_in_total.store(0, Ordering::SeqCst);
        return;
    }
    if ctx.shared.count_in_remaining.load(Ordering::Relaxed) > 0 {
        return;
    }
    state.rec.precount = None;
    super::rcu_tempo(ctx.tempo_map, |tm| {
        tm.metronome_enabled = pc.restore_metronome
    });
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

pub(crate) fn begin_recording_stream(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    start_sample: SamplePos,
) {
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
                device: t.input_device_name.load_full().map(|a| (*a).clone()),
                port: t.input_port(),
                mono: t.mono(),
            })
            .collect()
    };

    if armed_tracks.is_empty() {
        return;
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
        ctx.sample_rate,
        desired_channels,
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

    // Open a cycle-record session when loop-record mode is armed and a
    // real loop range is active. Each loop seam then rolls the in-progress
    // capture into a distinct take (see `poll_loop_record_seam`); without
    // it a looped recording keeps the legacy single-clip behaviour.
    state.loop_record_session = if state.rec.loop_record
        && state.rec.loop_enabled
        && state.rec.loop_out > state.rec.loop_in
    {
        Some(LoopRecordSession {
            slot: TimelineRange::from_bounds(state.rec.loop_in, state.rec.loop_out),
            pass_index: 0,
            groups: std::collections::HashMap::new(),
        })
    } else {
        None
    };
}

pub(crate) fn handle_pause(ctx: &HandlerCtx, state: &mut HandlerState) {
    let was_recording = ctx.shared.recording.load(Ordering::SeqCst);
    let was_playing = ctx.shared.playing.load(Ordering::Relaxed);
    ctx.shared.playing.store(false, Ordering::SeqCst);
    ctx.shared.recording.store(false, Ordering::SeqCst);
    cancel_precount(ctx, state);

    if was_recording {
        if state.loop_record_session.is_some() {
            // Cycle-record: emit the trailing pass as its own take instead
            // of the legacy single trimmed clip.
            finalize_loop_record_pass(ctx, state, false);
        } else {
            state
                .rec
                .finalize_recording(ctx.sample_rate, ctx.clips.as_ref(), ctx.event_tx);
        }
        state.rec.input_stream = None;
    }
    panic_all_instrument_plugins(ctx);
    super::midi::close_open_recordings(ctx, state);
    // close_open_recordings bails when no recording is active, so call
    // all-notes-off directly to silence hardware synths driven by the
    // timeline.
    state.midi_hw.midi_outputs.all_notes_off_everywhere();
    if was_playing {
        super::midi::clock_send_stop(state);
    }
}

pub(crate) fn handle_stop(ctx: &HandlerCtx, state: &mut HandlerState) {
    let was_recording = ctx.shared.recording.load(Ordering::SeqCst);
    let was_playing = ctx.shared.playing.load(Ordering::Relaxed);
    ctx.shared.playing.store(false, Ordering::SeqCst);
    ctx.shared.recording.store(false, Ordering::SeqCst);
    ctx.shared.playhead.store(0, Ordering::SeqCst);
    cancel_precount(ctx, state);

    if was_recording {
        if state.loop_record_session.is_some() {
            // Cycle-record: emit the trailing pass as its own take instead
            // of the legacy single trimmed clip.
            finalize_loop_record_pass(ctx, state, false);
        } else {
            state
                .rec
                .finalize_recording(ctx.sample_rate, ctx.clips.as_ref(), ctx.event_tx);
        }
        state.rec.input_stream = None;
    }

    panic_all_instrument_plugins(ctx);
    super::midi::close_open_recordings(ctx, state);
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
/// silence the plugin's tail. If the audio thread is mid-process now,
/// the next loop seam (or the next playback) will run the queued
/// NoteOffs, which is acoustically equivalent to what a one-block
/// silent pass here achieved.
fn panic_all_instrument_plugins(ctx: &HandlerCtx) {
    let tracks_guard = ctx.tracks.read();
    let plugins_guard = ctx.plugins.read();
    let mut silent_l = [0.0f32; 64];
    let mut silent_r = [0.0f32; 64];
    for track in tracks_guard.values() {
        if track.track_type.accepts_midi() {
            if let Some(&inst_id) = track.plugins().first() {
                if let Some(mutex) = plugins_guard.get(&inst_id) {
                    if let Some(mut inst) = mutex.try_lock() {
                        inst.0.all_notes_off();
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

pub(crate) fn handle_set_bpm(ctx: &HandlerCtx, bpm: f32) {
    super::rcu_tempo(ctx.tempo_map, |tm| tm.bpm = bpm.clamp(20.0, 999.0));
}

pub(crate) fn handle_set_tempo_events(
    ctx: &HandlerCtx,
    tempo: Vec<crate::types::TempoPoint>,
    signature: Vec<crate::types::SignaturePoint>,
) {
    let sample_rate = ctx.sample_rate;
    super::rcu_tempo(ctx.tempo_map, |tm| {
        if let Some(first) = tempo.first() {
            tm.bpm = first.bpm;
        }
        tm.tempo_points = tempo;
        tm.signature_points = signature;
        tm.rebuild_bar_table(sample_rate);
    });
}

pub(crate) fn handle_set_time_signature(ctx: &HandlerCtx, numerator: u8, denominator: u8) {
    super::rcu_tempo(ctx.tempo_map, |tm| {
        tm.numerator = numerator.max(1);
        tm.denominator = denominator.max(1);
    });
}

pub(crate) fn handle_set_metronome_enabled(ctx: &HandlerCtx, enabled: bool) {
    super::rcu_tempo(ctx.tempo_map, |tm| tm.metronome_enabled = enabled);
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

/// Toggle cycle-record (loop-record) mode. Stored on the recording state
/// so [`begin_recording_stream`] opens a [`LoopRecordSession`] when the
/// next record starts inside an active loop range.
pub(crate) fn handle_set_loop_record_mode(state: &mut HandlerState, on: bool) {
    state.rec.loop_record = on;
}

/// Detect a loop wrap during a cycle-record run and roll the finished pass.
///
/// Runs on the engine control thread every iteration. `last_playhead`
/// carries the previous-iteration position across calls; when the playhead
/// has moved backwards (the audio thread wrapped `loop_out` → `loop_in`)
/// while recording with an open [`LoopRecordSession`], the just-completed
/// pass is finalized into one take per armed track and a fresh capture is
/// started for the next pass.
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
    finalize_loop_record_pass(ctx, state, true);
}

/// Finalize the current cycle-record pass into one take per armed track
/// (an audio clip or a MIDI note set) and emit `AudioEvent::TakeCaptured`
/// for each. With `reopen` a fresh capture is started for the next pass
/// and `pass_index` is advanced; without it (transport stop) the session
/// is torn down after this trailing pass.
pub(crate) fn finalize_loop_record_pass(ctx: &HandlerCtx, state: &mut HandlerState, reopen: bool) {
    let Some(project_dir) = state.project_dir.clone() else {
        return;
    };
    let (slot, pass_index) = match state.loop_record_session.as_ref() {
        Some(s) => (s.slot, s.pass_index),
        None => return,
    };
    let audio_dir = project_dir.join("audio");

    // Pass 0's audio writer started where the user punched in; later passes
    // start at the loop boundary the seam wrapped to.
    let clip_start = if pass_index == 0 {
        state.rec.start_sample
    } else {
        slot.start
    };

    // -- Audio takes --
    let rolled = state.rec.roll_audio_pass(
        ctx.sample_rate,
        clip_start,
        ctx.clips.as_ref(),
        &audio_dir,
        &mut state.next_clip_id,
        reopen,
    );
    for take in rolled {
        let group_id = loop_record_group_for(state, take.track_id);
        let _ = ctx.event_tx.send(AudioEvent::TakeCaptured {
            group_id,
            track_id: take.track_id,
            slot,
            pass_index,
            content: TakeContent::Audio {
                clip_ref: take.clip_id,
            },
        });
    }

    // -- MIDI takes (instrument tracks) --
    let midi_takes = super::midi::capture_loop_record_midi_pass(ctx, state, slot.end());
    for (track_id, notes) in midi_takes {
        let group_id = loop_record_group_for(state, track_id);
        let _ = ctx.event_tx.send(AudioEvent::TakeCaptured {
            group_id,
            track_id,
            slot,
            pass_index,
            content: TakeContent::Midi { notes },
        });
    }

    if reopen {
        if let Some(s) = state.loop_record_session.as_mut() {
            s.pass_index += 1;
        }
    } else {
        state.loop_record_session = None;
    }
}

/// Stable take-group id for `track_id` within the active cycle-record run,
/// allocated on first use so every pass of a track shares one group.
fn loop_record_group_for(state: &mut HandlerState, track_id: TrackId) -> TakeGroupId {
    let next = &mut state.next_take_group_id;
    let session = state
        .loop_record_session
        .as_mut()
        .expect("loop-record session present");
    *session.groups.entry(track_id).or_insert_with(|| {
        let id = *next;
        *next += 1;
        id
    })
}
