//! Installing and reaping the *rendered audio* clips of a vocal lane.
//! The mirror of [`vocal_midi_install`](super::vocal_midi_install) on
//! the audio side, split out of `vocal_render` (ba todo #1259).
//!
//! The epoch check that discards a stale render and the swap that
//! replaces the previous take stay next to each other: the old take is
//! torn down only as the new one is installed, never when the render is
//! queued, so it keeps playing while the lane is `rendering` or `stale`
//! (vocal-mcp-control-and-quality.md §7, D22).

use resonance_audio::types::{ClipId, TrackId};

use crate::compose::VocalTakeLayout;

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

    let take = VocalTakeLayout {
        wav_path,
        clip_name,
        trim_start_frames,
        trim_end_frames,
        pre_roll_frames,
        lead_ticks,
        unit_keys: unit_keys.into(),
    };
    // The previous take played until now (spec §7, D22); it goes as the
    // new one arrives, placement by placement (a placement deleted while
    // the render ran took its clip with it: `purge_placement_outputs`).
    for (placement_id, _queued_start) in placements {
        install_take_at(r, definition_id, track_id, placement_id, &take);
    }
    r.compose.vocal_audio.takes.insert((definition_id, track_id), take);
    super::vocal_render_state::note_render_installed(r, (definition_id, track_id));
    true
}

/// Lay `take` at one placement of the lane's section, replacing the take
/// installed there. A placement that no longer exists is skipped.
fn install_take_at(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    placement_id: u64,
    take: &VocalTakeLayout,
) {
    use resonance_audio::types::AudioCommand;

    // The placement was deleted while the render ran — installing
    // audio for it would resurrect the section's vocal (VIEW-04).
    let Some(placement) = r.compose.find_placement(placement_id) else {
        return;
    };
    // …or it moved (a drag, bars inserted before it): place the audio
    // from where the section starts *now*, not where it started when
    // the render was queued (code review VIEW-19).
    let section_start = r.tempo_map.bar_to_sample(placement.start_bar);
    let note_sample = crate::compose::vocal_svs::vocal_audio_start(
        &r.tempo_map,
        section_start,
        take.lead_ticks,
        r.sample_rate,
    );
    // The first note's onset consonants sound before its beat; a
    // first note at sample 0 cannot start earlier, so what does not
    // fit is trimmed instead.
    let pre_roll_frames = take.pre_roll_frames;
    let start_sample = note_sample.saturating_sub(pre_roll_frames);
    let trim_start_frames =
        take.trim_start_frames + (pre_roll_frames - (note_sample - start_sample));
    let trim_end_frames = take.trim_end_frames;
    if let Some((old_id, old_path)) = r
        .compose
        .vocal_audio
        .clips
        .remove(&(definition_id, placement_id, track_id))
    {
        let _ = r.engine.send(AudioCommand::DeleteClip { clip_id: old_id });
        // Its mirror goes with it, as the new one's arrives below.
        r.clips.retain(|c| c.id != old_id);
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
        path: take.wav_path.clone(),
        name: take.clip_name.clone(),
        trim_start_frames,
        trim_end_frames,
    });
    // Mirror the clip now, trims and all, as the project load and a
    // pool placement do: the engine's `ClipImported` echo carries no
    // trims, so a mirror created from it alone would save the take
    // untrimmed — reopened, it played its leading pad, late — and a
    // reload could no longer find its first note. The echo fills in
    // the length and the waveform.
    r.clips.push(crate::state::ClipState {
        id: audio_clip_id,
        track_id,
        start_sample,
        duration_samples: 0,
        name: take.clip_name.clone(),
        total_frames: trim_start_frames + trim_end_frames,
        trim_start_frames,
        trim_end_frames,
        fade_in_frames: 0,
        fade_in_curve: Default::default(),
        fade_out_frames: 0,
        fade_out_curve: Default::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
        warp: Default::default(),
    });
    r.compose.vocal_audio.clips.insert(
        (definition_id, placement_id, track_id),
        (audio_clip_id, take.wav_path.clone()),
    );
    r.compose
        .vocal_audio
        .status
        .clip_unit_keys
        .insert(audio_clip_id, take.unit_keys.clone());
}

/// A section was placed again: lay the lane's current take at the new
/// placement too, so it sings there without a render (BUG-73). The caller
/// has checked that the take is current and that the new placement plays
/// at the tempo it was sung at.
///
/// The layout is the one the take was installed with
/// ([`VocalAudioRegistry::takes`](crate::compose::state::VocalAudioRegistry::takes))
/// while its WAV is the one installed; otherwise (after a reload) it is
/// read back off an installed clip that starts after sample 0 — one at
/// sample 0 may have had its pre-roll trimmed, which hides how long the
/// pre-roll was. Returns `false` when neither works: the placement stays
/// without a take and the lane reads `stale` until a render.
pub(super) fn lay_take_at_new_placement(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    placement_id: u64,
) -> bool {
    let installed: Vec<(u64, ClipId, std::path::PathBuf)> = r
        .compose
        .vocal_audio
        .clips
        .iter()
        .filter(|((d, p, t), _)| *d == definition_id && *t == track_id && *p != placement_id)
        .map(|((_, p, _), (clip, path))| (*p, *clip, path.clone()))
        .collect();
    if installed.is_empty() {
        return false;
    }
    let recorded = r
        .compose
        .vocal_audio
        .takes
        .get(&(definition_id, track_id))
        .filter(|t| installed.iter().all(|(_, _, path)| *path == t.wav_path))
        .cloned();
    let Some(take) = recorded.or_else(|| take_from_installed_clip(r, track_id, &installed)) else {
        return false;
    };
    install_take_at(r, definition_id, track_id, placement_id, &take);
    true
}

/// Read a take's layout back off one of its installed clips — the
/// inverse of [`install_take_at`] for a clip whose start was not clamped
/// at sample 0. A rendered take's leading trim and pre-roll add up to the
/// render's pad (`vocal_svs::pad_frames`), which the reload's owner match
/// (`ComposeState::rebuild_vocal_audio_clips`) relies on too; a clip that
/// does not sit where that puts its first note is not used.
fn take_from_installed_clip(
    r: &crate::Resonance,
    track_id: TrackId,
    installed: &[(u64, ClipId, std::path::PathBuf)],
) -> Option<VocalTakeLayout> {
    let pad = crate::compose::vocal_svs::pad_frames(r.sample_rate);
    installed.iter().find_map(|(placement_id, clip_id, path)| {
        let placement = r.compose.find_placement(*placement_id)?;
        let clip = r.clips.iter().find(|c| c.id == *clip_id)?;
        if clip.start_sample == 0 || clip.trim_start_frames > pad {
            return None;
        }
        let lead_ticks =
            super::vocal_melody::lane_lead_ticks(r, placement.definition_id, track_id)?;
        let note_sample = crate::compose::vocal_svs::vocal_audio_start(
            &r.tempo_map,
            r.tempo_map.bar_to_sample(placement.start_bar),
            lead_ticks,
            r.sample_rate,
        );
        let pre_roll_frames = pad - clip.trim_start_frames;
        if (clip.start_sample + pre_roll_frames).abs_diff(note_sample) > 2 {
            return None;
        }
        let unit_keys = r.compose.vocal_audio.status.clip_unit_keys.get(clip_id)?.clone();
        Some(VocalTakeLayout {
            wav_path: path.clone(),
            clip_name: clip.name.clone(),
            trim_start_frames: clip.trim_start_frames,
            trim_end_frames: clip.trim_end_frames,
            pre_roll_frames,
            lead_ticks,
            unit_keys,
        })
    })
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
