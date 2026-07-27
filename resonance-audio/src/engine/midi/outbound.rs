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

    // Snapshot the tracks with hardware output configured. Cheap
    // scan; typical projects have a handful of instrument tracks.
    // Muted tracks are skipped so the user can silence an external
    // instrument by muting its track — and so a "bounce in place" run
    // can isolate the source by muting the others. Any held notes on a
    // newly-muted track still get their NoteOff because the held-notes
    // map is consulted unconditionally at the bottom of this function.
    let output_tracks: Vec<(TrackId, u8)> = {
        let tracks = ctx.tracks.read();
        tracks
            .values()
            .filter(|t| t.midi_output_device.load_full().is_some() && !t.muted())
            .map(|t| (t.id, t.midi_output_channel.unwrap_or(0)))
            .collect()
    };
    if output_tracks.is_empty() {
        state.midi_hw.midi_outbound_last_playhead = curr;
        return;
    }

    // First: NoteOn for any timeline note that starts in (last..curr].
    {
        let tempo = ctx.tempo_map.load();
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
                        state.midi_hw.midi_outputs.send_note_on(
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
                            .midi_hw
                            .midi_outbound_held
                            .insert((*track_id, note.note), (note_end, *channel));
                    }
                }
            }
        }
    }

    // Second: NoteOff for held notes whose end fell in `[last, curr)`.
    let to_off: Vec<((TrackId, u8), (u64, u8))> = state
        .midi_hw
        .midi_outbound_held
        .iter()
        .filter(|(_, (end, _))| *end >= last && *end < curr)
        .map(|(k, v)| (*k, *v))
        .collect();
    for ((tid, note), (_end, channel)) in to_off {
        state.midi_hw.midi_outbound_held.remove(&(tid, note));
        state.midi_hw.midi_outputs.send_note_off(tid, channel, note);
    }

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
