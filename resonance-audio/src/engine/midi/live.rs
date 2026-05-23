//! Live MIDI input handling: routes notes from the GUI / hardware into
//! the track's instrument plugin (immediate playback) and into a
//! recording clip (when the track is armed during playback). Also owns
//! the transport-stop hook that closes still-open recorded notes.

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use crate::midi_hardware::LiveMidiEvent;
use crate::types::*;

use super::super::thread::{HandlerCtx, HandlerState, RecordingMidiState};
use super::sample_to_abs_tick;

pub(crate) fn handle_send_note_on(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    note: u8,
    velocity: f32,
) {
    let channel = {
        let tracks_guard = ctx.tracks.read();
        let Some(track) = tracks_guard.get(&track_id) else {
            return;
        };
        if !track.track_type.accepts_midi() {
            return;
        }
        if let Some(&inst_id) = track.plugins().first() {
            let plugins_guard = ctx.plugins.read();
            if let Some(mutex) = plugins_guard.get(&inst_id) {
                // `try_lock` so a slow plugin process() doesn't stall
                // every other engine command queued behind us
                // (transport, peaks, recording drain, MIDI clock).
                // If contended, retry by reposting the command — the
                // plugin will be free again within an audio block.
                if let Some(mut inst) = mutex.try_lock() {
                    inst.0.queue_note_on(note, velocity, 0);
                } else {
                    let _ = ctx.cmd_tx_retry.send(AudioCommand::SendNoteOn {
                        track_id,
                        note,
                        velocity,
                    });
                    return;
                }
            }
        }
        track.midi_output_channel.unwrap_or(0)
    };
    let velocity_u8 = (velocity.clamp(0.0, 1.0) * 127.0).round() as u8;
    state
        .midi_hw
        .midi_outputs
        .send_note_on(track_id, channel, note, velocity_u8);
}

pub(crate) fn handle_send_note_off(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    note: u8,
) {
    let channel = {
        let tracks_guard = ctx.tracks.read();
        let Some(track) = tracks_guard.get(&track_id) else {
            return;
        };
        if !track.track_type.accepts_midi() {
            return;
        }
        if let Some(&inst_id) = track.plugins().first() {
            let plugins_guard = ctx.plugins.read();
            if let Some(mutex) = plugins_guard.get(&inst_id) {
                // `try_lock` — see comment in `handle_send_note_on`.
                if let Some(mut inst) = mutex.try_lock() {
                    inst.0.queue_note_off(note, 0);
                } else {
                    let _ = ctx
                        .cmd_tx_retry
                        .send(AudioCommand::SendNoteOff { track_id, note });
                    return;
                }
            }
        }
        track.midi_output_channel.unwrap_or(0)
    };
    state
        .midi_hw
        .midi_outputs
        .send_note_off(track_id, channel, note);
}

/// Dispatch a single drained `LiveMidiEvent`. Routes inbound notes
/// to the track's instrument plugin (live monitoring) and to a
/// recording clip (when the track is armed during playback). Thru
/// to the track's hardware MIDI output is handled inside
/// `handle_send_note_on/off`.
pub(crate) fn handle_live_midi_event(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    event: LiveMidiEvent,
) {
    match event {
        LiveMidiEvent::InboundNoteOn {
            track_id,
            note,
            velocity,
            arrival,
        } => {
            // handle_send_note_on already routes to plugin AND to
            // the configured MIDI output (Thru). Record-into-clip
            // happens separately; recording must not also re-emit
            // the note.
            handle_send_note_on(ctx, state, track_id, note, velocity);
            handle_record_midi_event(ctx, state, track_id, true, note, velocity, arrival);
        }
        LiveMidiEvent::InboundNoteOff {
            track_id,
            note,
            arrival,
        } => {
            handle_send_note_off(ctx, state, track_id, note);
            handle_record_midi_event(ctx, state, track_id, false, note, 0.0, arrival);
        }
    }
}

/// Append a live note event into the track's currently-recording
/// MIDI clip when the track is record-armed and transport is
/// playing. Lazily creates a clip on the first NoteOn.
///
/// `arrival` is the wall-clock instant the midir callback fired for
/// this event. The recorder uses it to rewind the playhead by the
/// engine-thread drain delay (typically ~16 ms) so the recorded
/// `start_tick` reflects the actual key-press moment instead of the
/// processing moment. Without this, every recorded MIDI note would
/// land one engine tick late.
pub(crate) fn handle_record_midi_event(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    is_note_on: bool,
    note: u8,
    velocity: f32,
    arrival: std::time::Instant,
) {
    if !ctx.shared.playing.load(Ordering::Relaxed) {
        return;
    }
    // Only record into instrument tracks that are armed.
    let armed = {
        let tracks = ctx.tracks.read();
        match tracks.get(&track_id) {
            Some(t) => t.track_type.accepts_midi() && t.record_armed(),
            None => return,
        }
    };
    if !armed {
        return;
    }

    // Press-time playhead = current playhead minus elapsed-since-arrival,
    // converted to samples. Capture both `now` and `playhead` close
    // together so the math reflects the same instant on both sides.
    let now = std::time::Instant::now();
    let playhead_now = ctx.shared.playhead.load(Ordering::Relaxed);
    let elapsed_secs = now.saturating_duration_since(arrival).as_secs_f64();
    let elapsed_samples = (elapsed_secs * ctx.sample_rate as f64) as u64;
    let press_sample = playhead_now.saturating_sub(elapsed_samples);
    let abs_tick = sample_to_abs_tick(&ctx.tempo_map.load(), press_sample, ctx.sample_rate);

    if is_note_on {
        // Manual entry-or-insert: the closure form would force a
        // disjoint borrow of `state.next_clip_id` that the borrow
        // checker can't always prove safe.
        let needs_new_clip = !state.midi_recording.contains_key(&track_id);
        if needs_new_clip {
            let clip_id = state.next_clip_id;
            state.next_clip_id += 1;
            // Lazy clip creation on the first note. Start the clip
            // exactly at the press-time sample so the first note has
            // start_tick = 0 *and* the clip lines up with where the
            // user actually started playing rather than where the
            // engine thread happened to wake up.
            let clip = MidiClip {
                id: clip_id,
                track_id,
                start_sample: press_sample,
                duration_ticks: 0,
                notes: Vec::new(),
                name: format!("MIDI Take {}", clip_id),
                trim_start_ticks: 0,
                trim_end_ticks: 0,
            };
            ctx.midi_clips.write().push(clip);
            let _ = ctx.event_tx.send(AudioEvent::MidiClipCreated {
                clip_id,
                track_id,
                start_sample: press_sample,
                duration_ticks: 0,
                name: format!("MIDI Take {}", clip_id),
                notes: Vec::new(),
                trim_start_ticks: 0,
                trim_end_ticks: 0,
            });
            state.midi_recording.insert(
                track_id,
                RecordingMidiState {
                    clip_id,
                    clip_start_tick: abs_tick,
                    open_notes: HashMap::new(),
                },
            );
        }
        let rec = state.midi_recording.get_mut(&track_id).expect("just inserted");
        let clip_id = rec.clip_id;
        let start_tick = abs_tick.saturating_sub(rec.clip_start_tick);

        // Same-key retrigger: if a NoteOn arrives while the same key
        // is already held, treat it as an implicit NoteOff for the
        // earlier press. Without this, the open_notes overwrite
        // would orphan the first note at duration_ticks = 0 and the
        // matching NoteOff would later close the wrong note.
        let prior_open_idx = rec.open_notes.remove(&note);
        if let Some(prior_idx) = prior_open_idx {
            let mut clips = ctx.midi_clips.write();
            if let Some(clip) = clips.iter_mut().find(|c| c.id == clip_id) {
                if let Some(prev) = clip.notes.get_mut(prior_idx) {
                    let prev_dur = start_tick.saturating_sub(prev.start_tick);
                    prev.duration_ticks = prev_dur;
                    let new_clip_dur = clip.duration_ticks.max(prev.start_tick + prev_dur);
                    clip.duration_ticks = new_clip_dur;
                    let _ = ctx.event_tx.send(AudioEvent::MidiNoteResized {
                        clip_id,
                        note_index: prior_idx,
                        new_duration_ticks: prev_dur,
                    });
                }
            }
        }

        let new_note = MidiNote {
            note,
            velocity,
            start_tick,
            duration_ticks: 0,
        };
        let mut clips = ctx.midi_clips.write();
        if let Some(clip) = clips.iter_mut().find(|c| c.id == clip_id) {
            // Recording always appends in time order, but a stale
            // out-of-order event from a midir thread could land
            // after a later one. Insert sorted so playback stays
            // deterministic.
            let pos = clip
                .notes
                .partition_point(|n| n.start_tick <= start_tick);
            clip.notes.insert(pos, new_note.clone());
            // Track open notes by index. partition_point inserted
            // at `pos`, so any prior open-note indices ≥ pos shift up.
            for idx in rec.open_notes.values_mut() {
                if *idx >= pos {
                    *idx += 1;
                }
            }
            rec.open_notes.insert(note, pos);
            // Grow the clip's logical duration so the timeline
            // keeps drawing it — the user sees the clip extend in
            // real time as they play.
            clip.duration_ticks = clip.duration_ticks.max(start_tick + 1);
            let _ = ctx
                .event_tx
                .send(AudioEvent::MidiNoteAdded { clip_id, note: new_note });
        }
    } else {
        // NoteOff — close the matching open note's duration. If we
        // never saw the matching NoteOn (out-of-order, or playback
        // started mid-press) drop silently.
        let Some(rec) = state.midi_recording.get_mut(&track_id) else {
            return;
        };
        let Some(idx) = rec.open_notes.remove(&note) else {
            return;
        };
        let clip_id = rec.clip_id;
        let mut clips = ctx.midi_clips.write();
        if let Some(clip) = clips.iter_mut().find(|c| c.id == clip_id) {
            if let Some(n) = clip.notes.get_mut(idx) {
                let duration = abs_tick
                    .saturating_sub(rec.clip_start_tick)
                    .saturating_sub(n.start_tick);
                n.duration_ticks = duration;
                let dur = n.duration_ticks;
                let note_index = idx;
                let _ = ctx.event_tx.send(AudioEvent::MidiNoteResized {
                    clip_id,
                    note_index,
                    new_duration_ticks: dur,
                });
                clip.duration_ticks = clip.duration_ticks.max(n.start_tick + dur);
            }
        }
    }
}

/// Close any still-open recorded notes (e.g. user pressed Stop with
/// keys held down) and clear per-track recording state. Called from
/// the transport-stop handler.
pub(crate) fn close_open_recordings(ctx: &HandlerCtx, state: &mut HandlerState) {
    if state.midi_recording.is_empty() {
        return;
    }
    let playhead = ctx.shared.playhead.load(Ordering::Relaxed);
    let abs_tick = sample_to_abs_tick(&ctx.tempo_map.load(), playhead, ctx.sample_rate);
    let mut clips = ctx.midi_clips.write();
    for rec in state.midi_recording.values() {
        let Some(clip) = clips.iter_mut().find(|c| c.id == rec.clip_id) else {
            continue;
        };
        for (_note, idx) in rec.open_notes.iter() {
            if let Some(n) = clip.notes.get_mut(*idx) {
                let duration = abs_tick
                    .saturating_sub(rec.clip_start_tick)
                    .saturating_sub(n.start_tick);
                n.duration_ticks = duration;
            }
        }
    }
    drop(clips);
    state.midi_recording.clear();
    // Send All Notes Off on every output port so a hardware synth
    // doesn't sustain anything we'd lose on the engine side.
    state.midi_hw.midi_outputs.all_notes_off_everywhere();
}
