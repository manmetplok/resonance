//! Playhead and loop control commands (command-palette.md §5.2, §6):
//! every seek target resolved in one place, and the loop-point setter with
//! its snap / swap / clamp rules.
//!
//! All positions resolve through the GUI-side [`TempoMap`], so bar and beat
//! nudges are meter-aware: they land on exactly the lines the ruler draws.

use iced::Task;
use resonance_audio::types::{AudioCommand, TempoMap};

use crate::message::Message;
use crate::Resonance;

/// Where a [`TransportMessage::SeekTo`](super::transport::TransportMessage::SeekTo)
/// moves the playhead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekTarget {
    /// Sample 0.
    ProjectStart,
    /// The end of the last clip, section placement or marker.
    ProjectEnd,
    /// `loop_in`, whether or not the loop is enabled.
    LoopStart,
    /// `loop_out`.
    LoopEnd,
    /// Whole bars; an off-grid playhead first snaps to the bar line in the
    /// direction of travel.
    NudgeBars(i32),
    /// Whole beats of the signature in force (a 7/8 beat is an eighth);
    /// an off-grid playhead first snaps to the beat line ahead.
    NudgeBeats(i32),
    /// The nearest section-placement start before the playhead.
    PrevSection,
    /// The nearest section-placement start after the playhead.
    NextSection,
    /// The start of a 1-based bar, optionally at a 1-based beat.
    Bar { bar: u32, beat: u32 },
}

/// Which loop edge [`TransportMessage::SetLoopPoint`](super::transport::TransportMessage::SetLoopPoint)
/// moves to the playhead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopEdge {
    Start,
    End,
}

/// Two bar-table samples closer than this are the same grid line (the
/// table rounds each bar start to a whole sample).
const GRID_TOLERANCE: u64 = 2;

/// Sample of `beat` (0-based) in 0-based `bar`, also past the end of the
/// bar table, where the bar is divided evenly.
fn beat_sample(map: &TempoMap, bar: u32, beat: u32, sample_rate: u32) -> u64 {
    if let Some(s) = map.beat_sample_in_bar(bar as usize, beat, sample_rate) {
        if (bar as usize) < map.bar_count() {
            return s;
        }
    }
    let start = map.bar_to_sample(bar);
    let end = map.bar_to_sample(bar + 1);
    let beats = map.numerator_at_bar(bar).max(1) as u64;
    start + (end.saturating_sub(start)) * beat as u64 / beats
}

/// Every beat line in bars `from..=to`, ascending.
fn beat_lines(map: &TempoMap, from: u32, to: u32, sample_rate: u32) -> Vec<u64> {
    let mut lines = Vec::new();
    for bar in from..=to {
        for beat in 0..map.numerator_at_bar(bar).max(1) as u32 {
            lines.push(beat_sample(map, bar, beat, sample_rate));
        }
    }
    lines
}

/// The bar line strictly after `pos`.
fn next_bar_line(map: &TempoMap, pos: u64, sample_rate: u32) -> u64 {
    let (bar, _) = map.sample_to_bar(pos, sample_rate);
    let mut next = map.bar_to_sample(bar + 1);
    if next <= pos + GRID_TOLERANCE {
        next = map.bar_to_sample(bar + 2);
    }
    next
}

/// The bar line strictly before `pos` (0 at the start).
fn prev_bar_line(map: &TempoMap, pos: u64, sample_rate: u32) -> u64 {
    let (bar, _) = map.sample_to_bar(pos, sample_rate);
    let start = map.bar_to_sample(bar);
    if start + GRID_TOLERANCE < pos {
        start
    } else {
        map.bar_to_sample(bar.saturating_sub(1)).min(start)
    }
}

fn next_beat_line(map: &TempoMap, pos: u64, sample_rate: u32) -> u64 {
    let (bar, _) = map.sample_to_bar(pos, sample_rate);
    beat_lines(map, bar, bar + 1, sample_rate)
        .into_iter()
        .find(|&s| s > pos + GRID_TOLERANCE)
        .unwrap_or_else(|| map.bar_to_sample(bar + 2))
}

fn prev_beat_line(map: &TempoMap, pos: u64, sample_rate: u32) -> u64 {
    let (bar, _) = map.sample_to_bar(pos, sample_rate);
    beat_lines(map, bar.saturating_sub(1), bar, sample_rate)
        .into_iter()
        .rev()
        .find(|&s| s + GRID_TOLERANCE < pos)
        .unwrap_or(0)
}

/// Step `n` times with `forward` / `back`.
fn step(pos: u64, n: i32, mut forward: impl FnMut(u64) -> u64, mut back: impl FnMut(u64) -> u64) -> u64 {
    let mut p = pos;
    for _ in 0..n.unsigned_abs() {
        p = if n > 0 { forward(p) } else { back(p) };
    }
    p
}

/// Section-placement starts, ascending.
fn section_starts(r: &Resonance) -> Vec<u64> {
    let mut starts: Vec<u64> = r
        .compose
        .placements
        .iter()
        .map(|p| r.tempo_map.bar_to_sample(p.start_bar))
        .collect();
    starts.sort_unstable();
    starts.dedup();
    starts
}

/// The end of the last clip, MIDI clip, section placement or marker.
pub(crate) fn project_end(r: &Resonance) -> u64 {
    let clips = r.clips.iter().map(|c| c.start_sample + c.duration_samples);
    let midi = r.midi_clips.iter().map(|c| {
        r.tempo_map
            .tick_to_abs_sample(c.start_sample, c.duration_ticks, r.sample_rate)
    });
    let sections = r.compose.placements.iter().filter_map(|p| {
        r.compose
            .find_definition(p.definition_id)
            .map(|d| r.tempo_map.bar_to_sample(p.start_bar + d.length_bars))
    });
    let markers = r
        .markers
        .markers
        .iter()
        .map(|m| m.end_sample.unwrap_or(m.start_sample).max(m.start_sample));
    clips.chain(midi).chain(sections).chain(markers).max().unwrap_or(0)
}

/// Where `target` puts the playhead, or `None` when it has nowhere to go
/// (no section after the playhead, bar 0).
pub fn resolve_seek(r: &Resonance, target: SeekTarget) -> Option<u64> {
    let map = &r.tempo_map;
    let sr = r.sample_rate;
    let pos = r.transport.playhead;
    let sample = match target {
        SeekTarget::ProjectStart => 0,
        SeekTarget::ProjectEnd => project_end(r),
        SeekTarget::LoopStart => r.transport.loop_in,
        SeekTarget::LoopEnd => r.transport.loop_out,
        SeekTarget::NudgeBars(n) => step(
            pos,
            n,
            |p| next_bar_line(map, p, sr),
            |p| prev_bar_line(map, p, sr),
        ),
        SeekTarget::NudgeBeats(n) => step(
            pos,
            n,
            |p| next_beat_line(map, p, sr),
            |p| prev_beat_line(map, p, sr),
        ),
        SeekTarget::PrevSection => section_starts(r)
            .into_iter()
            .rev()
            .find(|&s| s + GRID_TOLERANCE < pos)?,
        SeekTarget::NextSection => section_starts(r)
            .into_iter()
            .find(|&s| s > pos + GRID_TOLERANCE)?,
        SeekTarget::Bar { bar, beat } => {
            let bar0 = bar.checked_sub(1)?;
            let beat0 = beat.saturating_sub(1);
            if beat0 >= map.numerator_at_bar(bar0).max(1) as u32 {
                return None;
            }
            beat_sample(map, bar0, beat0, sr)
        }
    };
    Some(sample)
}

/// Move the playhead to `target` through the ordinary seek path (the
/// engine seeks live while playing).
pub(crate) fn seek_to(r: &mut Resonance, target: SeekTarget) -> Task<Message> {
    if let Some(sample) = resolve_seek(r, target) {
        let _ = r.engine.send(AudioCommand::SeekTo(sample));
        r.transport.playhead = sample;
    }
    Task::none()
}

/// Move one loop edge to the playhead, snapped to the visible grid (D6).
/// A crossing pushes the other edge one bar away (clamped at 0), so the
/// range is never empty. Does not enable the loop.
pub(crate) fn set_loop_point(r: &mut Resonance, edge: LoopEdge) -> Task<Message> {
    let at = crate::view::timeline::snap_sample_to_grid_tempo(
        r.transport.playhead,
        r.transport.bpm,
        r.transport.time_sig_num,
        r.sample_rate,
        r.viewport.zoom,
        &r.tempo_map,
    );
    // One bar = the bar the edge lands in (meter-aware).
    let bar_len = |map: &TempoMap, sample: u64| {
        let (bar, _) = map.sample_to_bar(sample, r.sample_rate);
        map.bar_to_sample(bar + 1).saturating_sub(map.bar_to_sample(bar)).max(1)
    };
    let len_after = bar_len(&r.tempo_map, at);
    let len_before = bar_len(&r.tempo_map, at.saturating_sub(1));
    let t = &mut r.transport;
    if !t.loop_range_set {
        // No range yet: the other edge starts where this one lands, so the
        // crossing rule below makes it a one-bar loop.
        t.loop_in = at;
        t.loop_out = at;
    }
    match edge {
        LoopEdge::Start => {
            t.loop_in = at;
            if t.loop_in >= t.loop_out {
                t.loop_out = at + len_after;
            }
        }
        LoopEdge::End => {
            t.loop_out = at;
            if t.loop_out <= t.loop_in {
                t.loop_in = at.saturating_sub(len_before);
                if t.loop_out <= t.loop_in {
                    // At zero there is nothing before the end to push to.
                    t.loop_out = t.loop_in + len_after;
                }
            }
        }
    }
    t.loop_range_set = true;
    let _ = r.engine.send(AudioCommand::SetLoopRange {
        enabled: r.transport.loop_enabled,
        loop_in: r.transport.loop_in,
        loop_out: r.transport.loop_out,
    });
    Task::none()
}
