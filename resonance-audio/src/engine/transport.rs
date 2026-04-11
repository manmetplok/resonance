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

pub(crate) fn handle_record(ctx: &HandlerCtx, state: &mut HandlerState) {
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

    let source_name: Option<String> =
        armed_tracks.iter().find_map(|info| info.device.clone());

    let ring_size = super::RECORDING_RING_SIZE;
    let ring = ringbuf::HeapRb::<f32>::new(ring_size);
    use ringbuf::traits::Split;
    let (prod, cons) = ring.split();
    state.rec.ring_consumer = Some(cons);

    state.rec.start_sample = ctx.shared.playhead.load(Ordering::SeqCst);
    for info in &armed_tracks {
        state.rec.buffers.insert(
            info.track_id,
            crate::recording::TrackRecordingBuf {
                data: Vec::with_capacity(
                    ctx.sample_rate as usize * 2 * super::RECORDING_PREALLOC_SECONDS,
                ),
                input_port: info.port,
                mono: info.mono,
            },
        );
    }
    ctx.shared.recording.store(true, Ordering::SeqCst);

    match platform::build_input_stream(
        source_name.as_deref(),
        Arc::clone(ctx.shared),
        Some(prod),
        Arc::clone(ctx.monitor_prod),
        ctx.buf_frames,
        ctx.quantum,
    ) {
        Ok((stream, in_sr, in_ch)) => {
            state.rec.input_stream = Some(stream);
            state.rec.input_sample_rate = in_sr;
            state.rec.input_channels = in_ch;
            ctx.shared.input_channels.store(in_ch, Ordering::Release);

            let _ = ctx.event_tx.send(AudioEvent::RecordingStarted {
                start_sample: state.rec.start_sample,
            });
        }
        Err(e) => {
            ctx.shared.recording.store(false, Ordering::SeqCst);
            ctx.shared.input_channels.store(0, Ordering::Release);
            state.rec.buffers.clear();
            state.rec.ring_consumer = None;
            let _ = ctx
                .event_tx
                .send(AudioEvent::Error(format!("Failed to start recording: {}", e)));
        }
    }
}

pub(crate) fn handle_pause(ctx: &HandlerCtx, state: &mut HandlerState) {
    let was_recording = ctx.shared.recording.load(Ordering::SeqCst);
    ctx.shared.playing.store(false, Ordering::SeqCst);
    ctx.shared.recording.store(false, Ordering::SeqCst);

    if was_recording {
        state.rec.finalize_recording(
            ctx.sample_rate,
            &mut state.next_clip_id,
            ctx.clips.as_ref(),
            ctx.event_tx,
        );
        state.rec.input_stream = None;
    }
}

pub(crate) fn handle_stop(ctx: &HandlerCtx, state: &mut HandlerState) {
    let was_recording = ctx.shared.recording.load(Ordering::SeqCst);
    ctx.shared.playing.store(false, Ordering::SeqCst);
    ctx.shared.recording.store(false, Ordering::SeqCst);
    ctx.shared.playhead.store(0, Ordering::SeqCst);

    if was_recording {
        state.rec.finalize_recording(
            ctx.sample_rate,
            &mut state.next_clip_id,
            ctx.clips.as_ref(),
            ctx.event_tx,
        );
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

    let _ = ctx.event_tx.send(AudioEvent::Stopped);
}

pub(crate) fn handle_seek_to(ctx: &HandlerCtx, pos: u64) {
    ctx.shared.playhead.store(pos, Ordering::SeqCst);
}

pub(crate) fn handle_set_bpm(ctx: &HandlerCtx, bpm: f32) {
    ctx.tempo_map.write().bpm = bpm.clamp(20.0, 999.0);
}

pub(crate) fn handle_set_time_signature(
    ctx: &HandlerCtx,
    numerator: u8,
    denominator: u8,
) {
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
