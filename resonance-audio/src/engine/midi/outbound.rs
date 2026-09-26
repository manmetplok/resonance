//! Timeline → hardware MIDI output scheduling. Once per engine-thread
//! iteration (~16 ms) we look at the playhead delta `[last, curr)` and
//! emit NoteOn for any timeline note that started in the window plus
//! NoteOff for any held note that ended in it. The
//! [`outbound_step_start`] helper classifies discontinuities (loop
//! wrap, seek) so it can be unit-tested without spinning up the engine.

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use indexmap::IndexMap;
use resonance_common::device_definition::MidiBinding;
use resonance_common::{lane_value_to_binding_value, AutomationTarget};

use crate::engine::AutomationLanes;
use crate::midi_hardware::MidiOutputRegistry;
use crate::types::*;

use super::super::thread::{HandlerCtx, HandlerState};

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

/// Sink for the NoteOn/NoteOff messages emitted by the timeline →
/// hardware scheduler. The real [`MidiOutputRegistry`] forwards to its
/// note primitives; a capturing fake stands in for tests so
/// [`emit_outbound_notes`] — including the Recorded-span gating (doc
/// #257) — can be exercised without opening a hardware port.
pub trait OutboundNoteSink {
    /// Emit a NoteOn for `track_id` on `channel`.
    fn note_on(&mut self, track_id: TrackId, channel: u8, note: u8, velocity: u8);
    /// Emit a NoteOff for `track_id` on `channel`.
    fn note_off(&mut self, track_id: TrackId, channel: u8, note: u8);
}

impl OutboundNoteSink for MidiOutputRegistry {
    #[inline]
    fn note_on(&mut self, track_id: TrackId, channel: u8, note: u8, velocity: u8) {
        self.send_note_on(track_id, channel, note, velocity);
    }
    #[inline]
    fn note_off(&mut self, track_id: TrackId, channel: u8, note: u8) {
        self.send_note_off(track_id, channel, note);
    }
}

/// Per-track snapshot entry for one outbound poll step: which channel
/// the track emits on and whether the Recorded playback source gates
/// its notes over take-covered spans (doc #257).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutboundTrack {
    pub track_id: TrackId,
    pub channel: u8,
    /// `true` when the track's playback source is `Recorded` and it is
    /// not record-armed: notes starting inside a take-covered span are
    /// skipped and held notes are released on span entry. A record-armed
    /// track stays fully live so a punch-in still drives the hardware.
    pub gate_recorded: bool,
}

/// Snapshot the tracks with hardware MIDI output configured, resolving
/// each one's Recorded-span gating flag. Muted tracks are skipped so the
/// user can silence an external instrument by muting its track — and so
/// a "bounce in place" run can isolate the source by muting the others.
/// Pure over the tracks map so the flag derivation (`Recorded` + not
/// armed ⇒ gated) is unit-testable.
pub fn outbound_track_snapshot(tracks: &IndexMap<TrackId, Track>) -> Vec<OutboundTrack> {
    tracks
        .values()
        .filter(|t| t.midi_output_device.load_full().is_some() && !t.muted())
        .map(|t| OutboundTrack {
            track_id: t.id,
            channel: t.midi_output_channel.unwrap_or(0),
            gate_recorded: t.playback_source() == resonance_common::PlaybackSource::Recorded
                && !t.record_armed(),
        })
        .collect()
}

/// One hardware message scheduled inside a single poll window. The
/// window's NoteOns and NoteOffs are collected into one list and emitted
/// in timeline order rather than "every NoteOn, then every NoteOff", so a
/// pitch that repeats inside the window always releases before it
/// retriggers (see [`emit_outbound_notes`]).
struct PendingMsg {
    /// Timeline sample position this message belongs to.
    pos: u64,
    is_note_on: bool,
    track_id: TrackId,
    channel: u8,
    note: u8,
    /// NoteOn velocity; unused for a NoteOff.
    velocity: u8,
    /// For a NoteOn, the note's end position — what goes into `held`. For
    /// a NoteOff, the `held` end it was scheduled from, used as a
    /// staleness guard when an overlapping NoteOn re-armed the key first.
    end: u64,
}

/// Emit the hardware NoteOn/NoteOff messages for one forward poll window
/// `[last, curr)`. Pure core of [`poll_timeline_to_midi_output`] —
/// discontinuity classification (loop wrap / seek / stop) stays with the
/// caller, which drains `held` before calling this for a fresh segment.
///
/// Messages are emitted in timeline order, with a NoteOff sorting ahead of
/// a NoteOn at the same position, and a NoteOn for a pitch that is still
/// held releases it first. Both rules exist because the wire has to stay
/// strictly 1:1 on/off per pitch: a hardware synth that receives two
/// NoteOns for one key allocates two voices and only releases one of them
/// on the single NoteOff, leaving a note droning until the next panic.
/// Back-to-back repeats of the same pitch (note N ending exactly where
/// note N+1 starts) land in one ~16 ms poll window, so this is the common
/// case, not an edge case.
///
/// Recorded-span gating (doc #257), per track with `gate_recorded`:
///
/// 1. **Span entry releases held notes.** When the window overlaps any
///    take-covered span ([`audio_clip_covers`]), every note held on that
///    track is NoteOff'd *before* this window's NoteOns are considered —
///    a note sustained across the span boundary must not keep ringing on
///    the hardware over the take. Running the drain first means a note
///    that starts later in the same window, past the span's end, still
///    fires and survives. (While the playhead travels inside a span the
///    overlap keeps holding, but the held map stays empty, so the drain
///    is a no-op.)
/// 2. **Covered NoteOns are skipped.** A note whose start position lies
///    inside a covered span never fires — including when the window has
///    already moved past the span's end (no stale NoteOns on exit).
/// 3. Everything not covered behaves exactly live (the defined fallback
///    for "no take exists here"), and `gate_recorded: false` tracks are
///    byte-identical to the pre-mode behaviour.
#[allow(clippy::too_many_arguments)]
pub fn emit_outbound_notes<S: OutboundNoteSink, C: std::borrow::Borrow<MidiClip>>(
    output_tracks: &[OutboundTrack],
    midi_clips: &[C],
    audio_clips: &[AudioClip],
    tempo: &TempoMap,
    sample_rate: u32,
    last: u64,
    curr: u64,
    held: &mut HashMap<(TrackId, u8), (u64, u8)>,
    sink: &mut S,
) {
    // 1) Recorded gating: entering (or being inside) a covered span
    // releases the track's held notes before any new NoteOns fire.
    for ot in output_tracks.iter().filter(|ot| ot.gate_recorded) {
        if !audio_clip_covers(audio_clips, ot.track_id, last, curr) {
            continue;
        }
        let to_release: Vec<((TrackId, u8), u8)> = held
            .iter()
            .filter(|((tid, _), _)| *tid == ot.track_id)
            .map(|(k, (_end, channel))| (*k, *channel))
            .collect();
        for ((tid, note), channel) in to_release {
            held.remove(&(tid, note));
            sink.note_off(tid, channel, note);
        }
    }

    // 2) Collect the NoteOffs already due from `held` — before any of this
    // window's NoteOns can overwrite their entries.
    let mut pending: Vec<PendingMsg> = held
        .iter()
        .filter(|(_, (end, _))| *end >= last && *end < curr)
        .map(|((tid, note), (end, channel))| PendingMsg {
            pos: *end,
            is_note_on: false,
            track_id: *tid,
            channel: *channel,
            note: *note,
            velocity: 0,
            end: *end,
        })
        .collect();

    // 3) NoteOn for any timeline note that starts in `[last, curr)`.
    for ot in output_tracks {
        let clips = midi_clips.iter().map(std::borrow::Borrow::<MidiClip>::borrow);
        for clip in clips.filter(|c| c.track_id == ot.track_id) {
            // Trim is in tick space relative to the clip; the
            // visible portion is `[trim_start, duration - trim_end]`.
            let visible_end_tick = clip.duration_ticks.saturating_sub(clip.trim_end_ticks);
            for note in &clip.notes {
                if note.start_tick < clip.trim_start_ticks || note.start_tick >= visible_end_tick
                {
                    continue;
                }
                // Notes are stored in tick space relative to the
                // clip, but `tick_to_abs_sample` projects from
                // `clip.start_sample`. Subtract `trim_start_ticks`
                // so a trimmed clip's first audible note lands
                // exactly at `clip.start_sample`.
                let rel_start = note.start_tick - clip.trim_start_ticks;
                let rel_end =
                    (note.start_tick + note.duration_ticks).min(visible_end_tick)
                        - clip.trim_start_ticks;
                let note_start =
                    tempo.tick_to_abs_sample(clip.start_sample, rel_start, sample_rate);
                let note_end = tempo.tick_to_abs_sample(clip.start_sample, rel_end, sample_rate);
                // Half-open interval `[last, curr)`: each
                // sample-position is owned by exactly one poll
                // step, so a note at the very first playhead
                // value (e.g. sample 0 on the first poll after
                // play) fires, and no note ever fires twice.
                if note_start < last || note_start >= curr {
                    continue;
                }
                // Recorded gating: a note starting inside a covered
                // span is played by the take, not re-sent to the
                // hardware.
                if ot.gate_recorded
                    && audio_clip_covers(
                        audio_clips,
                        ot.track_id,
                        note_start,
                        note_start + 1,
                    )
                {
                    continue;
                }
                // A note can be shorter than the poll window, so its own
                // end may fall inside it. Keep `end` strictly after
                // `note_start` so the pair never collapses onto one
                // position (where the NoteOff would sort ahead of its own
                // NoteOn and strand the note).
                let note_end = note_end.max(note_start + 1);
                let velocity_u8 = (note.velocity.clamp(0.0, 1.0) * 127.0).round() as u8;
                pending.push(PendingMsg {
                    pos: note_start,
                    is_note_on: true,
                    track_id: ot.track_id,
                    channel: ot.channel,
                    note: note.note,
                    velocity: velocity_u8,
                    end: note_end,
                });
                if note_end < curr {
                    pending.push(PendingMsg {
                        pos: note_end,
                        is_note_on: false,
                        track_id: ot.track_id,
                        channel: ot.channel,
                        note: note.note,
                        velocity: 0,
                        end: note_end,
                    });
                }
            }
        }
    }

    // 4) Emit in timeline order. `sort_by_key` is stable, so messages at
    // the same position keep the collection order above (per track, per
    // clip, per note) — and `false < true` puts a NoteOff ahead of a
    // NoteOn sharing its position, which is what makes a note ending
    // exactly where the next one starts release before it retriggers.
    pending.sort_by_key(|m| (m.pos, m.is_note_on));
    for msg in pending {
        let key = (msg.track_id, msg.note);
        if msg.is_note_on {
            // A pitch still sounding from an earlier, overlapping note is
            // released first: without this its NoteOff would be dropped
            // when the entry below overwrites it, and the synth would be
            // left holding a voice it never gets an off for.
            if let Some((_, held_channel)) = held.remove(&key) {
                sink.note_off(msg.track_id, held_channel, msg.note);
            }
            sink.note_on(msg.track_id, msg.channel, msg.note, msg.velocity);
            held.insert(key, (msg.end, msg.channel));
        } else if held.get(&key) == Some(&(msg.end, msg.channel)) {
            // Still describes the note this NoteOff was scheduled for. A
            // mismatch means an overlapping NoteOn earlier in this window
            // already released the key and re-armed it, so this one is
            // stale and would cut the new note short.
            held.remove(&key);
            sink.note_off(msg.track_id, msg.channel, msg.note);
        }
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
        if !state.midi_hw.midi_outbound_held.is_empty() {
            let drained: Vec<((TrackId, u8), (u64, u8))> =
                state.midi_hw.midi_outbound_held.drain().collect();
            for ((tid, note), (_end, channel)) in drained {
                state.midi_hw.midi_outputs.send_note_off(tid, channel, note);
            }
        }
        state.midi_hw.midi_outbound_last_playhead = ctx.shared.playhead.load(Ordering::Relaxed);
        return;
    }

    let curr = ctx.shared.playhead.load(Ordering::Relaxed);
    let last_raw = state.midi_hw.midi_outbound_last_playhead;
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
                state.midi_hw.midi_outbound_held.drain().collect();
            for ((tid, note), (_end, channel)) in drained {
                state.midi_hw.midi_outputs.send_note_off(tid, channel, note);
            }
            match rewound {
                Some(loop_in) => loop_in,
                None => {
                    state.midi_hw.midi_outbound_last_playhead = curr;
                    return;
                }
            }
        }
    };
    if curr == last {
        return;
    }

    // Snapshot the tracks with hardware output configured (cheap scan;
    // typical projects have a handful of instrument tracks) and resolve
    // each one's Recorded-span gating flag. Any held notes on a
    // newly-muted track still get their NoteOff because the held-notes
    // map is consulted unconditionally inside `emit_outbound_notes`.
    let output_tracks = outbound_track_snapshot(&ctx.tracks.read());
    if output_tracks.is_empty() {
        // Every output track disappeared (unassigned, deleted or muted)
        // while notes were sounding: release them here, because nothing
        // downstream will ever schedule their NoteOff again.
        let drained: Vec<((TrackId, u8), (u64, u8))> =
            state.midi_hw.midi_outbound_held.drain().collect();
        for ((tid, note), (_end, channel)) in drained {
            state.midi_hw.midi_outputs.send_note_off(tid, channel, note);
        }
        state.midi_hw.midi_outbound_last_playhead = curr;
        return;
    }

    let tempo = ctx.tempo_map.load();
    let graph = ctx.shared.graph.load();
    let audio_clips = ctx.clips.read();
    emit_outbound_notes(
        &output_tracks,
        &graph.midi_clips,
        &audio_clips,
        &tempo,
        ctx.sample_rate,
        last,
        curr,
        &mut state.midi_hw.midi_outbound_held,
        &mut state.midi_hw.midi_outputs,
    );

    state.midi_hw.midi_outbound_last_playhead = curr;
}

/// Sink for the CC/NRPN messages emitted by device-parameter automation.
///
/// The real [`MidiOutputRegistry`] forwards to its
/// [`send_control_change`](MidiOutputRegistry::send_control_change) /
/// [`send_nrpn`](MidiOutputRegistry::send_nrpn) primitives (ba todo #718,
/// E1). A capturing fake stands in for tests so
/// [`emit_device_param_automation`] can be exercised without opening a
/// hardware port — and so the live engine poll and the realtime-bounce
/// drive, which run the identical core, are provably equal.
pub trait DeviceParamMidiSink {
    /// Emit a 7-bit Control Change for `track_id` on `channel`.
    fn emit_cc(&mut self, track_id: TrackId, channel: u8, cc: u8, value: u8);
    /// Emit an NRPN (parameter MSB/LSB + data-entry) for `track_id`.
    fn emit_nrpn(
        &mut self,
        track_id: TrackId,
        channel: u8,
        msb: u8,
        lsb: u8,
        value: u16,
        fourteen_bit: bool,
    );
}

impl DeviceParamMidiSink for MidiOutputRegistry {
    #[inline]
    fn emit_cc(&mut self, track_id: TrackId, channel: u8, cc: u8, value: u8) {
        self.send_control_change(track_id, channel, cc, value);
    }
    #[inline]
    fn emit_nrpn(
        &mut self,
        track_id: TrackId,
        channel: u8,
        msb: u8,
        lsb: u8,
        value: u16,
        fourteen_bit: bool,
    ) {
        self.send_nrpn(track_id, channel, msb, lsb, value, fourteen_bit);
    }
}

/// Evaluate every enabled [`AutomationTarget::DeviceParam`] lane at `frame`,
/// map each normalized lane value onto its parameter's MIDI binding integer
/// via [`lane_value_to_binding_value`], and emit a CC or NRPN through `sink`
/// — but only when the binding integer changed against `last_values`, so a
/// slow sweep doesn't flood the port (architecture doc #201 §4, ba todo
/// #723; acceptance criterion 4).
///
/// This is the single source of device-param MIDI emission, shared by the
/// live engine poll ([`poll_device_param_automation`]) and the realtime
/// "bounce in place" drive (both advance the playhead and run the engine
/// loop). Given the same lane set, track params, and `frame` sequence it
/// produces an identical ordered message stream, which is the live↔bounce
/// parity guarantee. (The *offline* bounce path has no hardware output and
/// never reaches here.)
///
/// `last_values` is the per-track, per-`DeviceParam::id` memo of the last
/// integer sent. The unchanged path touches only `HashMap::get` (no
/// allocation); a changed value clones the param id once to update the memo
/// — matching the cadence-throttled, low-churn style of
/// [`poll_timeline_to_midi_output`].
pub fn emit_device_param_automation<S: DeviceParamMidiSink>(
    lanes: &AutomationLanes,
    tracks: &IndexMap<TrackId, Track>,
    frame: u64,
    last_values: &mut HashMap<TrackId, HashMap<String, u16>>,
    sink: &mut S,
) {
    for lane in lanes.values() {
        if !lane.enabled {
            continue;
        }
        let AutomationTarget::DeviceParam { track, param_id } = &lane.target else {
            continue;
        };
        let Some(track_state) = tracks.get(track) else {
            continue;
        };
        // Resolve the parameter's binding/range/curve from the track's
        // device-param map (ba todo #722, E2). A lane whose param id is no
        // longer in the map (preset changed) is skipped until it returns.
        let Some(param) = track_state.device_param(param_id) else {
            continue;
        };

        let value = lane_value_to_binding_value(&param, lane.sample(frame));

        // De-dupe: skip unchanged binding integers. Borrows the param id —
        // no allocation on the steady-state (unchanged) path.
        let unchanged = last_values
            .get(track)
            .and_then(|m| m.get(param_id.as_str()))
            == Some(&value);
        if unchanged {
            continue;
        }

        let channel = track_state.midi_output_channel.unwrap_or(0);
        let emitted = match param.binding {
            MidiBinding::Cc { cc } => {
                sink.emit_cc(*track, channel, cc, value as u8);
                true
            }
            MidiBinding::Nrpn {
                msb,
                lsb,
                fourteen_bit,
            } => {
                sink.emit_nrpn(*track, channel, msb, lsb, value, fourteen_bit);
                true
            }
            // RPN has no E1 emission primitive yet; #723's scope is CC/NRPN
            // (the bundled devices bind only those). Left unsent rather than
            // misrouted through the NRPN address controllers.
            MidiBinding::Rpn { .. } => false,
        };
        if emitted {
            last_values
                .entry(*track)
                .or_default()
                .insert(param_id.clone(), value);
        }
    }
}

/// Engine-loop hook: once per iteration, emit hardware CC/NRPN for any
/// device-parameter automation lane whose mapped value moved since the last
/// poll. Mirrors [`poll_timeline_to_midi_output`]'s throttle (engine
/// cadence, de-duped writes); runs during both live playback and the
/// realtime bounce drive, so a bounced render emits the same control stream
/// as live.
pub(crate) fn poll_device_param_automation(ctx: &HandlerCtx, state: &mut HandlerState) {
    if !ctx.shared.playing.load(Ordering::Relaxed) {
        // Stopped: forget the memo so the next Play re-sends each param's
        // current value from the (possibly new) playhead position.
        state.midi_hw.device_param_last.clear();
        return;
    }
    let frame = ctx.shared.playhead.load(Ordering::Relaxed);
    let tracks = ctx.tracks.read();
    emit_device_param_automation(
        &state.automation_lanes,
        &tracks,
        frame,
        &mut state.midi_hw.device_param_last,
        &mut state.midi_hw.midi_outputs,
    );
}
