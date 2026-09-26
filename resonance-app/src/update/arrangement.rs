//! Structural bar shifts: insert or remove bars, moving everything after
//! the cut (ba doc #275 P2).
//!
//! This is the one edit that has to touch every timeline collection at
//! once — audio clips, MIDI clips, section placements, markers,
//! automation lanes and the two global tracks — which is exactly why it
//! cannot be assembled from per-object calls: a client doing it by hand
//! has to move ~160 objects without missing one, and nothing tells it if
//! it did.
//!
//! Positions move MUSICALLY. Each absolute frame position is read as a
//! tick, displaced by the tick span of the affected bars, and converted
//! back — the same two-step [`crate::update::tempo_reanchor`] uses, and
//! literally its machinery. Ticks are tempo-independent, so a project
//! with tempo changes shifts onto the right beat rather than by a fixed
//! number of frames.
//!
//! What does NOT move is anything that starts before the cut, even when
//! it plays across it: a 4-bar pad starting at bar 5 keeps its position
//! and its length when 2 bars are inserted at bar 7. Stretching it
//! instead would be a different edit, and a silent one.
//!
//! # The tempo and signature tracks (ba todo #1388, doc #287)
//!
//! They move too, and for the same reason everything else does: a
//! section placement at bar 32 and a meter change at bar 32 are one
//! musical statement, so anchoring the meter to an absolute bar while
//! the music slides past it leaves the two disagreeing with nothing to
//! tell a client. Events are stored in BARS, so they move by exact
//! `± count` arithmetic with no tick round-trip.
//!
//! Three rules keep the global tracks well-formed across a shift:
//!
//! - **Bar 0 is pinned.** Both lists keep a protected opening event at
//!   bar 0 (`update::global_track` refuses to remove index 0), and every
//!   bar of the song needs a tempo and a meter in force. Inserting at
//!   1-based bar 1 therefore leaves the opening event alone — the new
//!   bars simply take the song's opening tempo and meter, which is also
//!   what the material they push later already had. This is the one
//!   place where an event and a section placement differ: a placement at
//!   bar 0 is content and moves, an event at bar 0 is the state the
//!   content is played in and stays.
//! - **An event inside a removed span is CLAMPED to the cut, not
//!   dropped.** The music that survives the cut was written under that
//!   event; dropping it would silently re-time or re-meter surviving
//!   music, which is precisely the failure mode this todo exists to fix.
//!   Clamping keeps the splice sounding like what it was spliced from.
//! - **A bar that ends up carrying two events keeps the LATER one.**
//!   Several events can clamp onto the cut, and an event just after the
//!   removed span lands on it too. The state in force when the surviving
//!   music starts is the last one, so the last one wins — and both lists
//!   stay one-event-per-bar, which `global.*` depends on (ba todo #1382:
//!   two events on a bar make that bar unaddressable).
//!
//! # Order, and why the shift is measured rather than assumed
//!
//! Moving a tempo event changes the very map the shift is computed
//! against, so the steps cannot be reordered: capture the cut span and
//! every musical anchor under the OLD map, move the events, rebuild the
//! map, and only then write absolute positions back under the NEW one.
//!
//! `TempoMap` ramps linearly between tempo points, so this is not a
//! bookkeeping detail. Move a 140 BPM event eight bars later and its
//! whole approach ramp is eight bars longer, which changes the length of
//! every bar between it and the previous event — including bars before
//! the cut. `shift_samples` is therefore MEASURED after the fact (where
//! did the cut point actually end up?) rather than assumed from the
//! pre-shift bar table, and material before the cut is re-anchored to
//! its own musical position, exactly as a tempo edit re-anchors it.

use resonance_audio::types::{AudioCommand, ClipId};

use super::tempo_reanchor::{musical_anchors, reanchor_to_tempo};
use crate::state::{SignatureEvent, TempoEvent};
use crate::Resonance;

/// One structural shift's outcome, as the control layer reports it.
#[derive(Debug, Clone, Default)]
pub struct ShiftOutcome {
    pub shift_samples: i64,
    pub audio_clips_moved: u32,
    pub midi_clips_moved: u32,
    pub placements_moved: u32,
    pub markers_moved: u32,
    pub automation_points_moved: u32,
    pub tempo_events_moved: u32,
    pub signature_events_moved: u32,
    /// Events superseded on the bar they were clamped onto — see the
    /// last-wins rule in the module docs. Always 0 for `insert_bars`.
    pub tempo_events_removed: u32,
    pub signature_events_removed: u32,
    pub clips_deleted: Vec<ClipId>,
    pub placements_deleted: Vec<u64>,
}

/// What a shift would destroy, without doing it — the confirmation
/// preview for `arrangement.remove_bars`.
#[derive(Debug, Clone, Default)]
pub struct ShiftCasualties {
    pub audio_clips: Vec<ClipId>,
    pub midi_clips: Vec<ClipId>,
    pub placements: Vec<u64>,
}

impl ShiftCasualties {
    pub fn is_empty(&self) -> bool {
        self.audio_clips.is_empty() && self.midi_clips.is_empty() && self.placements.is_empty()
    }
}

/// The sample span `[from, to)` of `count` bars starting at 1-based
/// `at_bar`, plus the tick span between them. `None` when the bars fall
/// outside the tempo map's table.
fn cut_span(r: &Resonance, at_bar: u32, count: u32) -> (u64, u64, u64) {
    // `bar_to_sample` is 0-indexed; the wire is 1-based.
    let start = r.tempo_map.bar_to_sample(at_bar.saturating_sub(1));
    let end = r.tempo_map.bar_to_sample(at_bar.saturating_sub(1) + count);
    let sr = r.sample_rate;
    let ticks = r
        .tempo_map
        .sample_to_abs_tick(end, sr)
        .saturating_sub(r.tempo_map.sample_to_abs_tick(start, sr));
    (start, end, ticks)
}

/// Everything that starts inside the bars `remove_bars` would delete.
pub fn removal_casualties(r: &Resonance, at_bar: u32, count: u32) -> ShiftCasualties {
    let (cut, end, _) = cut_span(r, at_bar, count);
    ShiftCasualties {
        audio_clips: r
            .clips
            .iter()
            .filter(|c| c.start_sample >= cut && c.start_sample < end)
            .map(|c| c.id)
            .collect(),
        midi_clips: r
            .midi_clips
            .iter()
            .filter(|c| c.start_sample >= cut && c.start_sample < end)
            .map(|c| c.id)
            .collect(),
        placements: r
            .compose
            .placements
            .iter()
            .filter(|p| p.start_bar + 1 >= at_bar && p.start_bar + 1 < at_bar + count)
            .map(|p| p.id)
            .collect(),
    }
}

/// Insert `count` bars at 1-based `at_bar`, moving everything at or
/// after it later.
pub fn insert_bars(r: &mut Resonance, at_bar: u32, count: u32) -> ShiftOutcome {
    let at = at_bar.saturating_sub(1);
    let sr = r.sample_rate;
    let cut = r.tempo_map.bar_to_sample(at);
    let cut_tick = r.tempo_map.sample_to_abs_tick(cut, sr);

    // The new bars carry the meter in force BEFORE the cut, because a
    // signature event sitting exactly on `at` moves later with the music
    // it was written for. Taking the tick span of the current bars
    // `at..at+count` instead would count a meter change that is about to
    // leave them, and everything after the cut would land off the grid
    // by the difference. `bar_len_ticks_at` is the one bar-length
    // definition in the codebase (ba doc #288).
    let meter_bar = at.saturating_sub(1);
    let delta_ticks = i64::from(count) * r.tempo_map.bar_len_ticks_at(meter_bar) as i64;

    shift(
        r,
        Cut {
            from_bar: at_bar,
            sample: cut,
            tick: cut_tick,
            delta_ticks,
            delta_bars: i64::from(count),
            events: EventShift::Insert { at, count },
        },
    )
}

/// Remove `count` bars at 1-based `at_bar`: anything starting inside the
/// span is deleted, everything after it moves earlier.
pub fn remove_bars(r: &mut Resonance, at_bar: u32, count: u32) -> ShiftOutcome {
    let casualties = removal_casualties(r, at_bar, count);
    // Placements first, through the one teardown every placement delete
    // shares: it drops their derived MIDI clips, installed vocal audio
    // and the compose maps pointing at them (FU-B2). What it removes is
    // skipped below rather than deleted twice.
    for placement_id in &casualties.placements {
        crate::update::compose::purge_placement_outputs(r, *placement_id);
        r.compose.placements.retain(|p| p.id != *placement_id);
    }
    for clip_id in &casualties.audio_clips {
        if !r.clips.iter().any(|c| c.id == *clip_id) {
            continue;
        }
        r.clips.retain(|c| c.id != *clip_id);
        let _ = r.engine.send(AudioCommand::DeleteClip { clip_id: *clip_id });
    }
    if !casualties.audio_clips.is_empty() {
        r.recompute_pool_usage(); // review VIEW-30
    }
    for clip_id in &casualties.midi_clips {
        if !r.midi_clips.iter().any(|c| c.id == *clip_id) {
            continue;
        }
        r.midi_clips.retain(|c| c.id != *clip_id);
        r.compose.vocal_audio.clip_lyrics.remove(clip_id);
        let _ = r
            .engine
            .send(AudioCommand::DeleteMidiClip { clip_id: *clip_id });
    }

    let at = at_bar.saturating_sub(1);
    let sr = r.sample_rate;
    let (_, end, ticks) = cut_span(r, at_bar, count);
    let end_tick = r.tempo_map.sample_to_abs_tick(end, sr);

    let mut outcome = shift(
        r,
        Cut {
            from_bar: at_bar + count,
            sample: end,
            tick: end_tick,
            delta_ticks: -(ticks as i64),
            delta_bars: -i64::from(count),
            events: EventShift::Remove { at, count },
        },
    );
    outcome.clips_deleted = casualties
        .audio_clips
        .iter()
        .chain(casualties.midi_clips.iter())
        .copied()
        .collect();
    outcome.placements_deleted = casualties.placements;
    outcome
}

/// What a shift does to the two bar-addressed global tracks. Both
/// directions are stated in 0-based bars, as the events themselves are.
#[derive(Debug, Clone, Copy)]
enum EventShift {
    /// Open `count` bars at `at`: every event from `at` on moves that
    /// many bars later.
    Insert { at: u32, count: u32 },
    /// Close `count` bars at `at`: events after the span move that many
    /// bars earlier, events INSIDE it clamp onto `at`.
    Remove { at: u32, count: u32 },
}

impl EventShift {
    /// Where an event written at 0-based `bar` belongs afterwards.
    fn moved_bar(self, bar: u32) -> u32 {
        match self {
            EventShift::Insert { at, count } if bar >= at => bar + count,
            EventShift::Remove { at, count } if bar >= at + count => bar - count,
            EventShift::Remove { at, .. } if bar >= at => at,
            _ => bar,
        }
    }
}

/// How one global track fared.
#[derive(Debug, Clone, Copy, Default)]
struct EventMoves {
    moved: u32,
    removed: u32,
}

/// A bar-addressed event on one of the two global tracks. Both lists are
/// sorted by bar and hold at most one event per bar (ba todo #1382).
trait BarEvent {
    fn bar(&self) -> u32;
    fn set_bar(&mut self, bar: u32);
}

impl BarEvent for TempoEvent {
    fn bar(&self) -> u32 {
        self.bar
    }
    fn set_bar(&mut self, bar: u32) {
        self.bar = bar;
    }
}

impl BarEvent for SignatureEvent {
    fn bar(&self) -> u32 {
        self.bar
    }
    fn set_bar(&mut self, bar: u32) {
        self.bar = bar;
    }
}

/// Carry one global track across the cut, preserving all three of its
/// invariants: sorted by bar, one event per bar, and an event at bar 0.
/// See the module docs for why bar 0 is pinned and why the later event
/// wins a collision.
fn carry_events<T: BarEvent>(events: &mut Vec<T>, shift: EventShift) -> EventMoves {
    let mut moves = EventMoves::default();
    for event in events.iter_mut() {
        // The song's opening tempo/meter is not content; it is the state
        // bar 0 is played in, and bar 0 always exists.
        if event.bar() == 0 {
            continue;
        }
        let moved = shift.moved_bar(event.bar());
        if moved != event.bar() {
            event.set_bar(moved);
            moves.moved += 1;
        }
    }

    // The mapping is monotone and the sort is stable, so events that
    // collided on a bar sit next to each other in their original order —
    // last is the one that was in force when the surviving music starts.
    events.sort_by_key(|e| e.bar());
    let before = events.len();
    let mut kept: Vec<T> = Vec::with_capacity(before);
    for event in events.drain(..) {
        if kept.last().map(BarEvent::bar) == Some(event.bar()) {
            kept.pop();
        }
        kept.push(event);
    }
    *events = kept;
    moves.removed = (before - events.len()) as u32;
    moves
}

/// One cut, in each of the three units the collections it moves are
/// stored in. Captured against the PRE-shift tempo map: the shift is
/// about to change that map, and everything here has to describe where
/// the cut was when the edit was asked for.
#[derive(Debug, Clone, Copy)]
struct Cut {
    /// 1-based bar from which section placements move.
    from_bar: u32,
    /// Sample position at or after which absolute positions move.
    sample: u64,
    /// The same point in ticks.
    tick: u64,
    /// How far absolute positions move, in ticks. Negative for a removal.
    delta_ticks: i64,
    /// How far bar-addressed objects move. Negative for a removal.
    delta_bars: i64,
    /// What happens to the two global tracks.
    events: EventShift,
}

/// Move every timeline object starting at or after the cut, carry the
/// global tracks with them, and rebuild the tempo map around the result.
///
/// Shared by both directions so insert and remove cannot drift apart:
/// the only difference between them is the sign, the deletion pass and
/// which way the global tracks fold.
fn shift(r: &mut Resonance, cut: Cut) -> ShiftOutcome {
    let sr = r.sample_rate;
    let mut out = ShiftOutcome::default();

    // 1. Read every absolute position as a musical one under the map
    //    that is STILL IN FORCE, then displace the ones after the cut.
    //    Nothing is written back yet: the map is about to change.
    let mut anchors = musical_anchors(r);
    let displaced = anchors.displace_from(r, cut.sample, cut.delta_ticks);
    out.audio_clips_moved = displaced.audio_clips;
    out.midi_clips_moved = displaced.midi_clips;
    out.markers_moved = displaced.markers;
    out.automation_points_moved = displaced.automation_points;

    // 2. Section placements are bar-based, so they move by whole bars —
    //    the arrangement's own units, and immune to rounding.
    for placement in r.compose.placements.iter_mut() {
        if placement.start_bar + 1 >= cut.from_bar {
            let moved = placement.start_bar as i64 + cut.delta_bars;
            placement.start_bar = moved.max(0) as u32;
            out.placements_moved += 1;
        }
    }

    // 3. The global tracks are bar-based too, and move with the music
    //    they describe (ba todo #1388).
    let tempo = carry_events(&mut r.tempo_events, cut.events);
    let signature = carry_events(&mut r.signature_events, cut.events);
    out.tempo_events_moved = tempo.moved;
    out.tempo_events_removed = tempo.removed;
    out.signature_events_moved = signature.moved;
    out.signature_events_removed = signature.removed;

    // 4. Rebuild the bar table and re-send the map: moving an event
    //    invalidates both the GUI's copy and the engine's.
    r.rebuild_and_send_tempo();

    // 5. Put every absolute position back at its musical position under
    //    the NEW map. Positions before the cut are included on purpose —
    //    a stretched tempo ramp re-times them even though they did not
    //    move musically, and leaving them in frames is the half-converted
    //    song `tempo_reanchor` exists to prevent (ba doc #275 P1.4).
    reanchor_to_tempo(r, anchors);
    resort_automation(r);

    // 6. How far things actually went. Measured against the rebuilt map
    //    rather than read off the pre-shift bar table, because moving a
    //    tempo event reshapes the ramp the bar table was built from: the
    //    honest answer is where the cut point itself ended up.
    let landed_tick = (cut.tick as i64 + cut.delta_ticks).max(0) as u64;
    let landed = r.tempo_map.tick_to_abs_sample(0, landed_tick, sr);
    out.shift_samples = landed as i64 - cut.sample as i64;

    // 7. The transport reads its tempo and meter off the map, and both
    //    can now differ at the playhead.
    sync_transport_to_map(r);

    out
}

/// Removing bars can pull a later breakpoint onto or past an earlier one
/// (points inside the removed span stay where they are). The engine keeps
/// lanes sorted, and so must the mirror.
fn resort_automation(r: &mut Resonance) {
    let targets: Vec<_> = r.automation.lanes.keys().cloned().collect();
    for target in targets {
        let Some(lane) = r.automation.lanes.get_mut(&target) else {
            continue;
        };
        if lane
            .points
            .windows(2)
            .all(|w| w[0].time_frames <= w[1].time_frames)
        {
            continue;
        }
        lane.points.sort_by_key(|p| p.time_frames);
        let lane = lane.clone();
        let _ = r.engine.send(AudioCommand::SetAutomationLane { lane });
    }
}

/// Re-read the tempo and meter at the playhead, the way the global-track
/// shelf does after an edit. Both are display state the engine mirrors,
/// so the time signature is only re-sent when it actually changed.
fn sync_transport_to_map(r: &mut Resonance) {
    r.sync_tempo_display();
    let (_, numerator, denominator) = r
        .tempo_map
        .tempo_at_sample(r.transport.playhead, r.sample_rate);
    if (numerator, denominator) == (r.transport.time_sig_num, r.transport.time_sig_den) {
        return;
    }
    r.transport.time_sig_num = numerator;
    r.transport.time_sig_den = denominator;
    let _ = r.engine.send(AudioCommand::SetTimeSignature {
        numerator,
        denominator,
    });
}
