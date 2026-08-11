//! Keep the arrangement on its bars when the tempo changes (ba doc #275
//! P1.4).
//!
//! The timeline stores two kinds of position. Musical ones — a MIDI
//! clip's `duration_ticks`, a section placement's `start_bar` — are
//! expressed against the tempo map and follow it for free. Absolute ones
//! — every `start_sample`, every automation breakpoint's `time_frames`,
//! the loop range, the playhead — are frames, and a tempo change moves
//! the grid out from under them.
//!
//! Left alone that produces a half-converted song: after 120 → 140 a MIDI
//! clip's length shrank correctly (768000 → 658286 frames) while its
//! start stayed where it was, so a clip written at bar 9 played at bar
//! 10.33 while the section placement naming it still said bar 9. Nothing
//! reported an error; the song length just read 17.33 bars for a 16-bar
//! song.
//!
//! So a tempo change is taken in two steps: read every absolute position
//! as a musical one against the OLD map ([`musical_anchors`]), then put
//! each back at the same musical position under the NEW map
//! ([`reanchor_to_tempo`]).
//!
//! What deliberately does NOT rescale is audio-clip *duration*: a
//! recorded take is real time, and stretching it is `clip.set_stretch`'s
//! job, not the tempo field's. Its start still moves, so the take stays
//! on the downbeat it was recorded against.

use resonance_audio::types::{AudioCommand, ClipId, SamplePos};
use resonance_common::AutomationTarget;

use crate::Resonance;

/// Every absolute timeline position in the project, read as ticks against
/// the tempo map in force when it was captured.
pub(crate) struct MusicalAnchors {
    audio_clips: Vec<(ClipId, u64)>,
    midi_clips: Vec<(ClipId, u64)>,
    automation: Vec<(AutomationTarget, Vec<u64>)>,
    markers: Vec<(u64, u64, Option<u64>)>,
    playhead: u64,
    loop_in: u64,
    loop_out: u64,
}

/// Read the project's absolute positions as ticks under the CURRENT map.
/// Call this before the tempo map is rebuilt.
pub(crate) fn musical_anchors(r: &Resonance) -> MusicalAnchors {
    let sr = r.sample_rate;
    let tick = |sample: u64| r.tempo_map.sample_to_abs_tick(sample, sr);
    MusicalAnchors {
        audio_clips: r.clips.iter().map(|c| (c.id, tick(c.start_sample))).collect(),
        midi_clips: r
            .midi_clips
            .iter()
            .map(|c| (c.id, tick(c.start_sample)))
            .collect(),
        automation: r
            .automation
            .lanes
            .iter()
            .map(|(target, lane)| {
                (
                    target.clone(),
                    lane.points.iter().map(|p| tick(p.time_frames)).collect(),
                )
            })
            .collect(),
        markers: r
            .markers
            .markers
            .iter()
            .map(|m| (m.id, tick(m.start_sample), m.end_sample.map(tick)))
            .collect(),
        playhead: tick(r.transport.playhead),
        loop_in: tick(r.transport.loop_in),
        loop_out: tick(r.transport.loop_out),
    }
}

/// Put every anchor back at its musical position under the NEW map,
/// mirroring the move locally and telling the engine about it.
///
/// Each entity is looked up by id rather than by index: an anchor whose
/// clip vanished between the two halves is simply dropped.
pub(crate) fn reanchor_to_tempo(r: &mut Resonance, anchors: MusicalAnchors) {
    let sr = r.sample_rate;
    // `tick_to_abs_sample` integrates tempo changes from a start sample;
    // anchored at 0 it is the inverse of `sample_to_abs_tick`.
    let sample_at = |r: &Resonance, tick: u64| -> SamplePos {
        r.tempo_map.tick_to_abs_sample(0, tick, sr)
    };

    for (clip_id, tick) in anchors.audio_clips {
        let start = sample_at(r, tick);
        let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) else {
            continue;
        };
        if clip.start_sample == start {
            continue;
        }
        clip.start_sample = start;
        let new_track_id = clip.track_id;
        let _ = r.engine.send(AudioCommand::MoveClip {
            clip_id,
            new_start_sample: start,
            new_track_id,
        });
    }

    for (clip_id, tick) in anchors.midi_clips {
        let start = sample_at(r, tick);
        let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) else {
            continue;
        };
        if clip.start_sample == start {
            continue;
        }
        clip.start_sample = start;
        let new_track_id = clip.track_id;
        let _ = r.engine.send(AudioCommand::MoveMidiClip {
            clip_id,
            new_start_sample: start,
            new_track_id,
        });
    }

    for (target, ticks) in anchors.automation {
        let frames: Vec<u64> = ticks.iter().map(|t| sample_at(r, *t)).collect();
        let Some(lane) = r.automation.lanes.get_mut(&target) else {
            continue;
        };
        if lane.points.len() != frames.len() {
            continue;
        }
        let mut changed = false;
        for (point, frame) in lane.points.iter_mut().zip(frames) {
            if point.time_frames != frame {
                point.time_frames = frame;
                changed = true;
            }
        }
        if changed {
            let lane = lane.clone();
            let _ = r.engine.send(AudioCommand::SetAutomationLane { lane });
        }
    }

    for (id, start_tick, end_tick) in anchors.markers {
        let start = sample_at(r, start_tick);
        let end = end_tick.map(|t| sample_at(r, t));
        if let Some(marker) = r.markers.markers.iter_mut().find(|m| m.id == id) {
            marker.start_sample = start;
            marker.end_sample = end;
        }
    }

    // The playhead and the loop range are musical too — auditioning bars
    // 9-17 must still audition bars 9-17 after a tempo change.
    let playhead = sample_at(r, anchors.playhead);
    if playhead != r.transport.playhead {
        r.transport.playhead = playhead;
        let _ = r.engine.send(AudioCommand::SeekTo(playhead));
    }
    let (loop_in, loop_out) = (
        sample_at(r, anchors.loop_in),
        sample_at(r, anchors.loop_out),
    );
    if (loop_in, loop_out) != (r.transport.loop_in, r.transport.loop_out) {
        r.transport.loop_in = loop_in;
        r.transport.loop_out = loop_out;
        let _ = r.engine.send(AudioCommand::SetLoopRange {
            enabled: r.transport.loop_enabled,
            loop_in,
            loop_out,
        });
    }
}
