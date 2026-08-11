//! Structural bar shifts: insert or remove bars, moving everything after
//! the cut (ba doc #275 P2).
//!
//! This is the one edit that has to touch every timeline collection at
//! once — audio clips, MIDI clips, section placements, markers and
//! automation lanes — which is exactly why it cannot be assembled from
//! per-object calls: a client doing it by hand has to move ~160 objects
//! without missing one, and nothing tells it if it did.
//!
//! Positions move MUSICALLY. Each absolute frame position is read as a
//! tick, shifted by the tick span of the affected bars, and converted
//! back (the same two-step `tempo_reanchor` uses). Ticks are
//! tempo-independent, so a project with tempo changes shifts onto the
//! right beat rather than by a fixed number of frames.
//!
//! What does NOT move is anything that starts before the cut, even when
//! it plays across it: a 4-bar pad starting at bar 5 keeps its position
//! and its length when 2 bars are inserted at bar 7. Stretching it
//! instead would be a different edit, and a silent one.

use resonance_audio::types::{AudioCommand, ClipId};

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
    let (cut, end, ticks) = cut_span(r, at_bar, count);
    let mut out = shift(r, at_bar, cut, ticks as i64, count as i64);
    out.shift_samples = (end - cut) as i64;
    out
}

/// Remove `count` bars at 1-based `at_bar`: anything starting inside the
/// span is deleted, everything after it moves earlier.
pub fn remove_bars(r: &mut Resonance, at_bar: u32, count: u32) -> ShiftOutcome {
    let casualties = removal_casualties(r, at_bar, count);
    for clip_id in &casualties.audio_clips {
        r.clips.retain(|c| c.id != *clip_id);
        let _ = r.engine.send(AudioCommand::DeleteClip { clip_id: *clip_id });
    }
    for clip_id in &casualties.midi_clips {
        r.midi_clips.retain(|c| c.id != *clip_id);
        let _ = r
            .engine
            .send(AudioCommand::DeleteMidiClip { clip_id: *clip_id });
    }
    for placement_id in &casualties.placements {
        r.compose.placements.retain(|p| p.id != *placement_id);
        r.compose
            .derived_clips
            .retain(|(_, placement, _), _| placement != placement_id);
    }

    let (cut, end, ticks) = cut_span(r, at_bar, count);
    let mut outcome = shift(r, at_bar + count, end, -(ticks as i64), -(count as i64));
    outcome.shift_samples = -((end - cut) as i64);
    outcome.clips_deleted = casualties
        .audio_clips
        .iter()
        .chain(casualties.midi_clips.iter())
        .copied()
        .collect();
    outcome.placements_deleted = casualties.placements;
    outcome
}

/// Move every timeline object starting at or after `cut` by `delta_ticks`
/// (and every section placement at or after 1-based `from_bar` by
/// `delta_bars`).
///
/// Shared by both directions so insert and remove cannot drift apart:
/// the only difference between them is the sign and the deletion pass.
fn shift(
    r: &mut Resonance,
    from_bar: u32,
    cut: u64,
    delta_ticks: i64,
    delta_bars: i64,
) -> ShiftOutcome {
    let sr = r.sample_rate;
    let mut out = ShiftOutcome::default();

    let moved_to = |r: &Resonance, start: u64| -> u64 {
        let tick = r.tempo_map.sample_to_abs_tick(start, sr) as i64;
        let shifted = (tick + delta_ticks).max(0) as u64;
        r.tempo_map.tick_to_abs_sample(0, shifted, sr)
    };

    // Audio clips.
    let audio: Vec<(ClipId, u64, u64)> = r
        .clips
        .iter()
        .filter(|c| c.start_sample >= cut)
        .map(|c| (c.id, c.track_id, moved_to(r, c.start_sample)))
        .collect();
    for (clip_id, track_id, start) in audio {
        if let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) {
            clip.start_sample = start;
        }
        let _ = r.engine.send(AudioCommand::MoveClip {
            clip_id,
            new_start_sample: start,
            new_track_id: track_id,
        });
        out.audio_clips_moved += 1;
    }

    // MIDI clips.
    let midi: Vec<(ClipId, u64, u64)> = r
        .midi_clips
        .iter()
        .filter(|c| c.start_sample >= cut)
        .map(|c| (c.id, c.track_id, moved_to(r, c.start_sample)))
        .collect();
    for (clip_id, track_id, start) in midi {
        if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
            clip.start_sample = start;
        }
        let _ = r.engine.send(AudioCommand::MoveMidiClip {
            clip_id,
            new_start_sample: start,
            new_track_id: track_id,
        });
        out.midi_clips_moved += 1;
    }

    // Section placements are bar-based, so they move by whole bars —
    // the arrangement's own units, and immune to rounding.
    for placement in r.compose.placements.iter_mut() {
        if placement.start_bar + 1 >= from_bar {
            let moved = placement.start_bar as i64 + delta_bars;
            placement.start_bar = moved.max(0) as u32;
            out.placements_moved += 1;
        }
    }

    // Markers.
    let marker_moves: Vec<(u64, u64, Option<u64>)> = r
        .markers
        .markers
        .iter()
        .filter(|m| m.start_sample >= cut)
        .map(|m| {
            (
                m.id,
                moved_to(r, m.start_sample),
                m.end_sample.map(|e| moved_to(r, e)),
            )
        })
        .collect();
    for (id, start, end) in marker_moves {
        if let Some(marker) = r.markers.markers.iter_mut().find(|m| m.id == id) {
            marker.start_sample = start;
            marker.end_sample = end;
            out.markers_moved += 1;
        }
    }

    // Automation breakpoints. A lane is republished whole, once, rather
    // than point by point.
    let targets: Vec<_> = r.automation.lanes.keys().cloned().collect();
    for target in targets {
        let Some(lane) = r.automation.lanes.get(&target) else {
            continue;
        };
        let moves: Vec<(usize, u64)> = lane
            .points
            .iter()
            .enumerate()
            .filter(|(_, p)| p.time_frames >= cut)
            .map(|(i, p)| (i, moved_to(r, p.time_frames)))
            .collect();
        if moves.is_empty() {
            continue;
        }
        let Some(lane) = r.automation.lanes.get_mut(&target) else {
            continue;
        };
        for (i, frames) in moves {
            lane.points[i].time_frames = frames;
            out.automation_points_moved += 1;
        }
        // Removing bars can pull a later point onto an earlier one; the
        // engine keeps lanes sorted, and so must the mirror.
        lane.points.sort_by_key(|p| p.time_frames);
        let lane = lane.clone();
        let _ = r.engine.send(AudioCommand::SetAutomationLane { lane });
    }

    out
}
