//! Instrument-track creation, MIDI clip CRUD, per-note editing, and
//! live-MIDI note-on/note-off passthrough to the track's instrument
//! plugin.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::midi_clock::MidiClockEvent;
use crate::midi_hardware::{enumerate_midi_inputs, enumerate_midi_outputs, LiveMidiEvent};
use crate::types::*;

use super::thread::{HandlerCtx, HandlerState, RecordingMidiState};

pub(crate) fn handle_add_instrument_track(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    id_hint: Option<TrackId>,
    name: Option<String>,
) {
    let id = id_hint.unwrap_or_else(|| {
        let i = state.next_track_id;
        state.next_track_id += 1;
        i
    });
    if id_hint.is_some() {
        state.next_track_id = state.next_track_id.max(id + 1);
    }
    let name = name.unwrap_or_else(|| format!("Instrument {}", id));
    let track = Track::with_type(id, name, TrackType::Instrument);
    ctx.tracks.write().insert(id, track);
    let _ = ctx
        .event_tx
        .send(AudioEvent::InstrumentTrackAdded { track_id: id });
}

pub(crate) fn handle_create_midi_clip(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    start_sample: u64,
    duration_ticks: u64,
    name: String,
) {
    let clip_id = state.next_clip_id;
    state.next_clip_id += 1;
    let clip = MidiClip {
        id: clip_id,
        track_id,
        start_sample,
        duration_ticks,
        notes: Vec::new(),
        name: name.clone(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    };
    ctx.midi_clips.write().push(clip);
    let _ = ctx.event_tx.send(AudioEvent::MidiClipCreated {
        clip_id,
        track_id,
        start_sample,
        duration_ticks,
        name,
        notes: Vec::new(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_load_midi_clip_direct(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    clip_id: ClipId,
    track_id: TrackId,
    start_sample: u64,
    duration_ticks: u64,
    notes: Vec<MidiNote>,
    name: String,
    trim_start_ticks: u64,
    trim_end_ticks: u64,
) {
    let clip = MidiClip {
        id: clip_id,
        track_id,
        start_sample,
        duration_ticks,
        notes: notes.clone(),
        name: name.clone(),
        trim_start_ticks,
        trim_end_ticks,
    };
    ctx.midi_clips.write().push(clip);
    state.next_clip_id = state.next_clip_id.max(clip_id + 1);
    let _ = ctx.event_tx.send(AudioEvent::MidiClipCreated {
        clip_id,
        track_id,
        start_sample,
        duration_ticks,
        name,
        notes,
        trim_start_ticks,
        trim_end_ticks,
    });
}

pub(crate) fn handle_move_midi_clip(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    new_start_sample: u64,
    new_track_id: TrackId,
) {
    let mut guard = ctx.midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.track_id = new_track_id;
    }
    let _ = ctx.event_tx.send(AudioEvent::MidiClipMoved {
        clip_id,
        new_start_sample,
        new_track_id,
    });
}

pub(crate) fn handle_trim_midi_clip(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    new_start_sample: u64,
    trim_start_ticks: u64,
    trim_end_ticks: u64,
) {
    let mut guard = ctx.midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.trim_start_ticks = trim_start_ticks;
        clip.trim_end_ticks = trim_end_ticks;
    }
    let _ = ctx.event_tx.send(AudioEvent::MidiClipTrimmed {
        clip_id,
        new_start_sample,
        trim_start_ticks,
        trim_end_ticks,
    });
}

pub(crate) fn handle_delete_midi_clip(ctx: &HandlerCtx, clip_id: ClipId) {
    ctx.midi_clips.write().retain(|c| c.id != clip_id);
    let _ = ctx.event_tx.send(AudioEvent::MidiClipDeleted { clip_id });
}

pub(crate) fn handle_add_midi_note(ctx: &HandlerCtx, clip_id: ClipId, note: MidiNote) {
    let mut guard = ctx.midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        let n = note.clone();
        // Insert sorted by start_tick
        let pos = clip
            .notes
            .partition_point(|n| n.start_tick <= note.start_tick);
        clip.notes.insert(pos, note);
        let _ = ctx
            .event_tx
            .send(AudioEvent::MidiNoteAdded { clip_id, note: n });
    }
}

pub(crate) fn handle_remove_midi_note(ctx: &HandlerCtx, clip_id: ClipId, note_index: usize) {
    let mut guard = ctx.midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        if note_index < clip.notes.len() {
            clip.notes.remove(note_index);
            let _ = ctx.event_tx.send(AudioEvent::MidiNoteRemoved {
                clip_id,
                note_index,
            });
        }
    }
}

pub(crate) fn handle_move_midi_note(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    note_index: usize,
    new_start_tick: u64,
    new_note: u8,
) {
    let mut guard = ctx.midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        if note_index < clip.notes.len() {
            clip.notes[note_index].start_tick = new_start_tick;
            clip.notes[note_index].note = new_note;
            // Re-sort
            clip.notes.sort_by_key(|n| n.start_tick);
            let _ = ctx.event_tx.send(AudioEvent::MidiNoteMoved {
                clip_id,
                note_index,
                new_start_tick,
                new_note,
            });
        }
    }
}

pub(crate) fn handle_resize_midi_note(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    note_index: usize,
    new_duration_ticks: u64,
) {
    let mut guard = ctx.midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        if note_index < clip.notes.len() {
            clip.notes[note_index].duration_ticks = new_duration_ticks;
            let _ = ctx.event_tx.send(AudioEvent::MidiNoteResized {
                clip_id,
                note_index,
                new_duration_ticks,
            });
        }
    }
}

pub(crate) fn handle_set_midi_note_velocity(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    note_index: usize,
    velocity: f32,
) {
    let mut guard = ctx.midi_clips.write();
    if let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) {
        if note_index < clip.notes.len() {
            clip.notes[note_index].velocity = velocity;
            let _ = ctx.event_tx.send(AudioEvent::MidiNoteVelocitySet {
                clip_id,
                note_index,
                velocity,
            });
        }
    }
}

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
        if track.track_type != TrackType::Instrument {
            return;
        }
        if let Some(&inst_id) = track.plugin_ids.first() {
            let plugins_guard = ctx.plugins.read();
            if let Some(mutex) = plugins_guard.get(&inst_id) {
                let mut inst = mutex.lock();
                inst.0.queue_note_on(note, velocity, 0);
            }
        }
        track.midi_output_channel.unwrap_or(0)
    };
    let velocity_u8 = (velocity.clamp(0.0, 1.0) * 127.0).round() as u8;
    state
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
        if track.track_type != TrackType::Instrument {
            return;
        }
        if let Some(&inst_id) = track.plugin_ids.first() {
            let plugins_guard = ctx.plugins.read();
            if let Some(mutex) = plugins_guard.get(&inst_id) {
                let mut inst = mutex.lock();
                inst.0.queue_note_off(note, 0);
            }
        }
        track.midi_output_channel.unwrap_or(0)
    };
    state.midi_outputs.send_note_off(track_id, channel, note);
}

// -----------------------------------------------------------------------------
// Hardware MIDI I/O
// -----------------------------------------------------------------------------

pub(crate) fn handle_list_midi_inputs(ctx: &HandlerCtx, state: &mut HandlerState) {
    let devices = enumerate_midi_inputs();
    // Always reconcile: a fresh connect attempt for a pending track
    // is cheap and the only way "unplug, replug" recovers without
    // user intervention. The unchanged-list dedupe below only
    // suppresses the GUI round-trip, not the reconnect attempt.
    state.midi_inputs.reconcile();
    if devices != state.last_midi_input_devices {
        state.last_midi_input_devices = devices.clone();
        let _ = ctx
            .event_tx
            .send(AudioEvent::MidiInputDevicesListed { devices });
    }
}

pub(crate) fn handle_list_midi_outputs(ctx: &HandlerCtx, state: &mut HandlerState) {
    let devices = enumerate_midi_outputs();
    if devices != state.last_midi_output_devices {
        state.last_midi_output_devices = devices.clone();
        let _ = ctx
            .event_tx
            .send(AudioEvent::MidiOutputDevicesListed { devices });
    }
}

pub(crate) fn handle_set_track_midi_input(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    device: Option<String>,
    channel: Option<u8>,
) {
    // Persist the desired config on the engine-side track for
    // subsequent saves and for the registry's reconnect-on-replug
    // path. Plain field write — only the engine thread reads it.
    {
        let mut tracks = ctx.tracks.write();
        if let Some(t) = tracks.get_mut(&track_id) {
            t.midi_input_device = device.clone();
            t.midi_input_channel = channel;
        }
    }
    if let Err(e) = state.midi_inputs.set_track_input(track_id, device, channel) {
        let _ = ctx.event_tx.send(AudioEvent::Error(e));
    }
}

pub(crate) fn handle_set_track_midi_output(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    track_id: TrackId,
    device: Option<String>,
    channel: Option<u8>,
) {
    // Mirror onto the engine-side track. The audio thread reads
    // `midi_output_device` via arc-swap (no lock), so the swap is
    // visible to the next mix block immediately even though the map
    // itself is held under a write lock for the channel update.
    {
        let mut tracks = ctx.tracks.write();
        if let Some(t) = tracks.get_mut(&track_id) {
            match &device {
                Some(name) => t.midi_output_device.store(Some(Arc::new(name.clone()))),
                None => t.midi_output_device.store(None),
            }
            t.midi_output_channel = channel;
        }
    }
    if let Err(e) = state.midi_outputs.set_track_output(track_id, device) {
        let _ = ctx.event_tx.send(AudioEvent::Error(e));
    }
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
            Some(t) => t.track_type == TrackType::Instrument && t.record_armed(),
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
    let abs_tick = sample_to_abs_tick(&ctx.tempo_map.read(), press_sample, ctx.sample_rate);

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
    let abs_tick = sample_to_abs_tick(&ctx.tempo_map.read(), playhead, ctx.sample_rate);
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
    state.midi_outputs.all_notes_off_everywhere();
}

/// Convert an absolute sample position to an absolute tick using
/// the engine's shared tempo map. Thin wrapper over
/// [`TempoMap::sample_to_abs_tick`].
fn sample_to_abs_tick(map: &TempoMap, sample_pos: u64, sample_rate: u32) -> u64 {
    map.sample_to_abs_tick(sample_pos, sample_rate)
}

/// Resolution of a single outbound poll step against the previous
/// `last_playhead`. Returned by [`outbound_step_start`].
#[derive(Debug, PartialEq, Eq)]
pub enum OutboundStep {
    /// Normal forward step. Emit notes in `[last, curr)` using the
    /// contained `last`.
    Continue(u64),
    /// Discontinuity (loop wrap, seek, scrub, transport restart).
    /// Caller must drain any outstanding held notes. If the inner
    /// option is `Some(loop_in)`, the discontinuity is a loop wrap
    /// and the caller should still emit notes in `[loop_in, curr)`.
    /// If `None`, it's a genuine seek/scrub and no notes fire
    /// retroactively this poll.
    Discontinuity(Option<u64>),
}

/// Decide where this poll should start emitting notes from. Pure
/// helper extracted from [`poll_timeline_to_midi_output`] so the
/// loop-wrap rewind logic can be unit-tested without spinning up the
/// full engine thread.
///
/// `max_normal_step` is hardcoded to one second (the engine polls at
/// ~60 Hz, so any apparent jump bigger than that has to be a seek or
/// loop wrap rather than the playhead simply advancing).
pub fn outbound_step_start(
    last_raw: u64,
    curr: u64,
    sample_rate: u32,
    looping: bool,
    loop_in: u64,
    loop_out: u64,
) -> OutboundStep {
    let max_normal_step = sample_rate as u64;
    let normal_step = curr >= last_raw && curr - last_raw < max_normal_step;
    if normal_step {
        return OutboundStep::Continue(last_raw);
    }
    // Loop wrap: backward jump while looping with `curr` in the loop
    // range. The audio thread snapped the playhead from `loop_out`
    // back to `loop_in` and advanced from there, so by the time we
    // poll, `curr` already sits past `loop_in`. Rewind `last` to
    // `loop_in` so the first note of the new iteration plays.
    if looping
        && curr < last_raw
        && loop_out > loop_in
        && curr >= loop_in
        && curr < loop_out
    {
        OutboundStep::Discontinuity(Some(loop_in))
    } else {
        OutboundStep::Discontinuity(None)
    }
}

/// Send hardware MIDI for any timeline note whose start/end fell in
/// `(last_playhead .. current_playhead]`, on tracks configured with
/// a MIDI output device. Runs once per engine-thread iteration
/// (~16 ms granularity).
///
/// On stop, on a backward jump, or on a forward jump >1 s (scrub or
/// seek) we emit NoteOff for everything we have outstanding and
/// reset the cursor; otherwise the next poll would either re-fire
/// every note since 0 or strand held notes. A loop wrap (backward
/// jump while looping with `curr` inside the loop range) is the one
/// discontinuity we *do* emit notes through — the cursor rewinds to
/// `loop_in` so the first note of the new iteration plays.
pub(crate) fn poll_timeline_to_midi_output(ctx: &HandlerCtx, state: &mut HandlerState) {
    let playing = ctx.shared.playing.load(Ordering::Relaxed);
    if !playing {
        // Transition to stopped: kill any outstanding hardware notes
        // so the synth doesn't sustain. Then snap our cursor to the
        // current playhead so the next Play resumes from there
        // rather than re-firing every note since the last position.
        if !state.midi_outbound_held.is_empty() {
            let drained: Vec<((TrackId, u8), (u64, u8))> =
                state.midi_outbound_held.drain().collect();
            for ((tid, note), (_end, channel)) in drained {
                state.midi_outputs.send_note_off(tid, channel, note);
            }
        }
        state.midi_outbound_last_playhead = ctx.shared.playhead.load(Ordering::Relaxed);
        return;
    }

    let curr = ctx.shared.playhead.load(Ordering::Relaxed);
    let last_raw = state.midi_outbound_last_playhead;
    let looping = ctx.shared.loop_enabled.load(Ordering::Relaxed);
    let lo = ctx.shared.loop_in.load(Ordering::Relaxed);
    let hi = ctx.shared.loop_out.load(Ordering::Relaxed);
    let last = match outbound_step_start(
        last_raw,
        curr,
        ctx.sample_rate,
        looping,
        lo,
        hi,
    ) {
        OutboundStep::Continue(last) => last,
        OutboundStep::Discontinuity(rewound) => {
            // Drop every held note from the previous segment before
            // emitting (or skipping) the new one.
            let drained: Vec<((TrackId, u8), (u64, u8))> =
                state.midi_outbound_held.drain().collect();
            for ((tid, note), (_end, channel)) in drained {
                state.midi_outputs.send_note_off(tid, channel, note);
            }
            match rewound {
                Some(loop_in) => loop_in,
                None => {
                    state.midi_outbound_last_playhead = curr;
                    return;
                }
            }
        }
    };
    if curr == last {
        return;
    }

    // Snapshot the tracks with hardware output configured. Cheap
    // scan; typical projects have a handful of instrument tracks.
    let output_tracks: Vec<(TrackId, u8)> = {
        let tracks = ctx.tracks.read();
        tracks
            .values()
            .filter(|t| t.midi_output_device.load_full().is_some())
            .map(|t| (t.id, t.midi_output_channel.unwrap_or(0)))
            .collect()
    };
    if output_tracks.is_empty() {
        state.midi_outbound_last_playhead = curr;
        return;
    }

    // First: NoteOn for any timeline note that starts in (last..curr].
    {
        let tempo = ctx.tempo_map.read();
        let clips = ctx.midi_clips.read();
        for (track_id, channel) in &output_tracks {
            for clip in clips.iter().filter(|c| c.track_id == *track_id) {
                // Trim is in tick space relative to the clip; the
                // visible portion is `[trim_start, duration - trim_end]`.
                let visible_end_tick = clip
                    .duration_ticks
                    .saturating_sub(clip.trim_end_ticks);
                for note in &clip.notes {
                    if note.start_tick < clip.trim_start_ticks
                        || note.start_tick >= visible_end_tick
                    {
                        continue;
                    }
                    // Notes are stored in tick space relative to the
                    // clip, but `tick_to_abs_sample` projects from
                    // `clip.start_sample`. Subtract `trim_start_ticks`
                    // so a trimmed clip's first audible note lands
                    // exactly at `clip.start_sample`.
                    let rel_start = note.start_tick - clip.trim_start_ticks;
                    let rel_end = (note.start_tick + note.duration_ticks)
                        .min(visible_end_tick)
                        - clip.trim_start_ticks;
                    let note_start =
                        tempo.tick_to_abs_sample(clip.start_sample, rel_start, ctx.sample_rate);
                    let note_end =
                        tempo.tick_to_abs_sample(clip.start_sample, rel_end, ctx.sample_rate);
                    // Half-open interval `[last, curr)`: each
                    // sample-position is owned by exactly one poll
                    // step, so a note at the very first playhead
                    // value (e.g. sample 0 on the first poll after
                    // play) fires, and no note ever fires twice.
                    if note_start >= last && note_start < curr {
                        let velocity_u8 =
                            (note.velocity.clamp(0.0, 1.0) * 127.0).round() as u8;
                        state.midi_outputs.send_note_on(
                            *track_id,
                            *channel,
                            note.note,
                            velocity_u8,
                        );
                        // If the same pitch is already held (e.g.
                        // overlapping notes on the same track), the
                        // earlier NoteOff time gets clobbered. Most
                        // hardware synths handle a second NoteOn on a
                        // held pitch as "retrigger", which matches
                        // what the user sees on the timeline.
                        state
                            .midi_outbound_held
                            .insert((*track_id, note.note), (note_end, *channel));
                    }
                }
            }
        }
    }

    // Second: NoteOff for held notes whose end fell in `[last, curr)`.
    let to_off: Vec<((TrackId, u8), (u64, u8))> = state
        .midi_outbound_held
        .iter()
        .filter(|(_, (end, _))| *end >= last && *end < curr)
        .map(|(k, v)| (*k, *v))
        .collect();
    for ((tid, note), (_end, channel)) in to_off {
        state.midi_outbound_held.remove(&(tid, note));
        state.midi_outputs.send_note_off(tid, channel, note);
    }

    state.midi_outbound_last_playhead = curr;
}

// -----------------------------------------------------------------------------
// MIDI clock master / slave
// -----------------------------------------------------------------------------

/// Convert an absolute sample position to an absolute MIDI clock tick
/// (24 PPQN). The internal tempo map's tick resolution is 480 PPQN
/// (`TICKS_PER_QUARTER_NOTE`), so dividing by 20 lands on the clock
/// resolution.
fn sample_to_clock_tick(map: &TempoMap, sample_pos: u64, sample_rate: u32) -> u64 {
    let abs_tick = map.sample_to_abs_tick(sample_pos, sample_rate);
    abs_tick / (TICKS_PER_QUARTER_NOTE / 24)
}

/// Convert an absolute sample position to a Song Position Pointer
/// value in MIDI beats (16th notes from song start).
fn sample_to_spp(map: &TempoMap, sample_pos: u64, sample_rate: u32) -> u16 {
    let abs_tick = map.sample_to_abs_tick(sample_pos, sample_rate);
    // 480 ticks per quarter ÷ 4 = 120 ticks per 16th note.
    let sixteenths = abs_tick / (TICKS_PER_QUARTER_NOTE / 4);
    sixteenths.min(0x3FFF) as u16
}

pub(crate) fn handle_set_midi_clock_output(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    device: Option<String>,
    enabled: bool,
) {
    if let Err(e) = state.midi_clock_sender.configure(device, enabled) {
        let _ = ctx.event_tx.send(AudioEvent::Error(e));
    }
}

pub(crate) fn handle_set_midi_clock_input(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    device: Option<String>,
    enabled: bool,
) {
    if let Err(e) = state.midi_clock_receiver.configure(device, enabled) {
        let _ = ctx.event_tx.send(AudioEvent::Error(e));
    }
    // Resetting the tempo tracker on every reconfigure avoids a stale
    // average being applied to a freshly opened device.
    state.midi_clock_tempo.reset();
    state.midi_clock_external_running = false;
    state.midi_clock_last_emitted_bpm = 0.0;
}

/// Catch the master clock up to the playhead. Always called from the
/// engine thread loop; bails cheaply when the master isn't enabled.
/// Driven both during playback (clock pulses follow the tempo map)
/// and while stopped (no pulses, but `last_clock_tick` stays in sync
/// with the playhead so a Continue resumes from the right spot).
pub(crate) fn poll_midi_clock_send(ctx: &HandlerCtx, state: &mut HandlerState) {
    if !state.midi_clock_sender.is_active() {
        return;
    }
    if !ctx.shared.playing.load(Ordering::Relaxed) {
        return;
    }
    let playhead = ctx.shared.playhead.load(Ordering::Relaxed);
    let clock_tick = sample_to_clock_tick(&ctx.tempo_map.read(), playhead, ctx.sample_rate);
    state.midi_clock_sender.poll_send_clock(clock_tick);
}

/// Emit a MIDI Start aligned to playhead 0, called from the transport
/// Play handler. Caller is responsible for deciding whether this is a
/// fresh start (playhead == 0) or a Continue (playhead > 0).
pub(crate) fn clock_send_start(state: &mut HandlerState) {
    state.midi_clock_sender.send_start();
}

pub(crate) fn clock_send_continue(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    sample_pos: u64,
) {
    if !state.midi_clock_sender.is_active() {
        return;
    }
    let clock_tick = sample_to_clock_tick(&ctx.tempo_map.read(), sample_pos, ctx.sample_rate);
    let spp = sample_to_spp(&ctx.tempo_map.read(), sample_pos, ctx.sample_rate);
    // Send SPP first so the receiver knows where to resume, then
    // Continue. This matches the convention most external gear
    // expects (Reason, Ableton, MPC, etc.).
    state.midi_clock_sender.send_song_position(spp, clock_tick);
    state.midi_clock_sender.send_continue(clock_tick);
}

pub(crate) fn clock_send_stop(state: &mut HandlerState) {
    state.midi_clock_sender.send_stop();
}

pub(crate) fn clock_send_song_position(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    sample_pos: u64,
) {
    if !state.midi_clock_sender.is_active() {
        return;
    }
    let clock_tick = sample_to_clock_tick(&ctx.tempo_map.read(), sample_pos, ctx.sample_rate);
    let spp = sample_to_spp(&ctx.tempo_map.read(), sample_pos, ctx.sample_rate);
    state.midi_clock_sender.send_song_position(spp, clock_tick);
}

/// Apply one drained inbound clock message to the transport and the
/// tempo tracker. Driving the transport from the engine thread keeps
/// the locking story simple: the same paths the GUI uses to start
/// playback are invoked here, so all the same invariants hold.
pub(crate) fn handle_midi_clock_event(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    event: MidiClockEvent,
) {
    if !state.midi_clock_receiver.is_active() {
        return;
    }
    match event {
        MidiClockEvent::Start { .. } => {
            state.midi_clock_tempo.reset();
            state.midi_clock_external_running = true;
            ctx.shared.playhead.store(0, Ordering::SeqCst);
            ctx.shared.playing.store(true, Ordering::SeqCst);
            let _ = ctx.event_tx.send(AudioEvent::PlayheadMoved(0));
            let _ = ctx.event_tx.send(AudioEvent::MidiClockStarted);
        }
        MidiClockEvent::Continue { .. } => {
            state.midi_clock_tempo.reset();
            state.midi_clock_external_running = true;
            ctx.shared.playing.store(true, Ordering::SeqCst);
            let _ = ctx.event_tx.send(AudioEvent::MidiClockContinued);
        }
        MidiClockEvent::Stop => {
            state.midi_clock_external_running = false;
            ctx.shared.playing.store(false, Ordering::SeqCst);
            // Mirror the standard Stop handler's all-notes-off so a
            // hardware synth doesn't sustain when the master halts.
            super::transport::handle_pause(ctx, state);
            let _ = ctx.event_tx.send(AudioEvent::MidiClockStopped);
        }
        MidiClockEvent::Clock { arrival } => {
            // Only react to BPM derivation while the external master
            // is in run state. Many devices send free-running clock
            // even when stopped; we don't want that smear into our
            // tempo while we're stationary.
            if !state.midi_clock_external_running {
                return;
            }
            if let Some(bpm) = state.midi_clock_tempo.observe(arrival) {
                if (bpm - state.midi_clock_last_emitted_bpm).abs() > 0.1 {
                    state.midi_clock_last_emitted_bpm = bpm;
                    ctx.tempo_map.write().bpm = bpm;
                    let _ = ctx
                        .event_tx
                        .send(AudioEvent::MidiClockTempoDetected { bpm });
                }
            }
        }
        MidiClockEvent::SongPosition { sixteenths } => {
            // Convert 16th notes to absolute samples through the tempo
            // map and seek there. SPP only legally arrives while
            // stopped, but we accept it during playback too because
            // some hardware sequencers do exactly that.
            let abs_tick = (sixteenths as u64) * (TICKS_PER_QUARTER_NOTE / 4);
            let sample = ctx.tempo_map.read().tick_to_abs_sample(
                0,
                abs_tick,
                ctx.sample_rate,
            );
            ctx.shared.playhead.store(sample, Ordering::SeqCst);
            let _ = ctx.event_tx.send(AudioEvent::PlayheadMoved(sample));
        }
    }
}
