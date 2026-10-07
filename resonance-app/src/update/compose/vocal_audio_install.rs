//! Installing and reaping the *rendered audio* clips of a vocal lane.
//! The mirror of [`vocal_midi_install`](super::vocal_midi_install) on
//! the audio side, split out of `vocal_render` (ba todo #1259).
//!
//! The epoch check that discards a stale render and the swap that
//! replaces the previous take stay next to each other: the old take is
//! torn down only as the new one is installed, never when the render is
//! queued, so it keeps playing while the lane is `rendering` or `stale`
//! (vocal-mcp-control-and-quality.md §7, D22).

use resonance_audio::types::TrackId;

use super::vocal_audio_io::{unlink_if_exists, unlink_superseded_take};

/// Apply the vocal audio render result: send `LoadClipFromWav` to the
/// engine for every snapshotted placement and remember the resulting
/// clip ids (+ path) so the next regen can tear them down cleanly.
///
/// Returns whether the render was **accepted** — `false` means the
/// epoch check found it superseded and its audio was discarded. The
/// caller must not resolve any control job off a discarded render: a
/// job ticked off a stale event reports `done` for audio that never
/// installed.
pub(super) fn handle_vocal_audio_ready(
    r: &mut crate::Resonance,
    data: crate::compose::messages::VocalAudioReadyData,
) -> bool {
    use resonance_audio::types::AudioCommand;

    let crate::compose::messages::VocalAudioReadyData {
        definition_id,
        track_id,
        wav_path,
        placements,
        clip_name,
        trim_start_frames,
        trim_end_frames,
        pre_roll_frames,
        lead_ticks,
        render_epoch,
        bpm: _,
        unit_keys,
    } = data;

    if render_epoch != current_render_epoch(r, definition_id, track_id) {
        unlink_if_exists(&wav_path);
        return false;
    }

    // The previous take played until now (spec §7, D22); it goes as the
    // new one arrives, placement by placement (a placement deleted while
    // the render ran took its clip with it: `purge_placement_outputs`).
    let unit_keys: std::sync::Arc<[u64]> = unit_keys.into();
    for (placement_id, _queued_start) in placements {
        // The placement was deleted while the render ran — installing
        // audio for it would resurrect the section's vocal (VIEW-04).
        let Some(placement) = r.compose.find_placement(placement_id) else {
            continue;
        };
        // …or it moved (a drag, bars inserted before it): place the audio
        // from where the section starts *now*, not where it started when
        // the render was queued (code review VIEW-19).
        let section_start = r.tempo_map.bar_to_sample(placement.start_bar);
        let note_sample = crate::compose::vocal_svs::vocal_audio_start(
            &r.tempo_map,
            section_start,
            lead_ticks,
            r.sample_rate,
        );
        // The first note's onset consonants sound before its beat; a
        // first note at sample 0 cannot start earlier, so what does not
        // fit is trimmed instead.
        let start_sample = note_sample.saturating_sub(pre_roll_frames);
        let trim_start_frames = trim_start_frames + (pre_roll_frames - (note_sample - start_sample));
        if let Some((old_id, old_path)) = r
            .compose
            .vocal_audio
            .clips
            .remove(&(definition_id, placement_id, track_id))
        {
            let _ = r.engine
                .send(AudioCommand::DeleteClip { clip_id: old_id });
            r.compose.vocal_audio.status.clip_unit_keys.remove(&old_id);
            // Never the saved project's `clip_<id>.wav`, nor a take
            // another placement still plays (FU-C1a).
            let installed = r.compose.vocal_audio.clips.values().map(|(_, p)| p);
            unlink_superseded_take(&old_path, installed);
        }

        let audio_clip_id = r.media.ids.clips.allocate();
        let _ = r.engine.send(AudioCommand::LoadClipFromWav {
            clip_id: audio_clip_id,
            track_id,
            start_sample,
            path: wav_path.clone(),
            name: clip_name.clone(),
            trim_start_frames,
            trim_end_frames,
        });
        r.compose.vocal_audio.clips.insert(
            (definition_id, placement_id, track_id),
            (audio_clip_id, wav_path.clone()),
        );
        r.compose
            .vocal_audio
            .status
            .clip_unit_keys
            .insert(audio_clip_id, unit_keys.clone());
    }
    super::vocal_render_state::note_render_installed(r, (definition_id, track_id));
    true
}

/// Record that the render carrying `render_epoch` has reported back
/// (success or failure) and, when it was stale, fail every control job
/// still waiting on the lane if no newer render is in flight to resolve
/// it (code review UPD-08).
///
/// A stale completion normally has a successor: a later request bumped
/// the epoch *and* queued a render, whose event will complete every job
/// covering the lane. But a restore (undo, project load) wipes
/// the epoch table, and a track/section delete bumps it without queuing
/// anything — then the stale event is the last one the lane will ever
/// see, and a `vocal.render` job waiting on it would hang until its
/// client gave up.
pub(super) fn settle_render_event(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    render_epoch: u64,
    accepted: bool,
) {
    let key = (definition_id, track_id);
    let in_flight = &mut r.compose.vocal_audio.in_flight_render;
    if in_flight.get(&key) == Some(&render_epoch) {
        in_flight.remove(&key);
    }
    if !accepted && !in_flight.contains_key(&key) {
        r.control.jobs.fail_vocal_lane(
            definition_id,
            track_id,
            "the vocal render was discarded: the project changed under it \
             (undo, project load, or the lane was removed) before it finished",
            None,
        );
    }
}

/// The lane's current render epoch — the snapshot a completion (success
/// or failure) must carry to be about the render that is actually in
/// flight, rather than one a later request superseded. `0` for a lane
/// that never rendered.
pub(super) fn current_render_epoch(
    r: &crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> u64 {
    r.compose
        .vocal_audio
        .render_epoch
        .get(&(definition_id, track_id))
        .copied()
        .unwrap_or(0)
}
