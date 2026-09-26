//! Installing and reaping the *rendered audio* clips of a vocal lane.
//! The mirror of [`vocal_midi_install`](super::vocal_midi_install) on
//! the audio side, split out of `vocal_render` (ba todo #1259).
//!
//! Both halves of the lane's clip lifecycle live here so the epoch
//! check that discards a stale render and the tear-down that precedes a
//! fresh one stay next to each other.

use resonance_audio::types::TrackId;

use super::vocal_audio_io::unlink_if_exists;

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
        render_epoch,
    } = data;

    if render_epoch != current_render_epoch(r, definition_id, track_id) {
        unlink_if_exists(&wav_path);
        return false;
    }

    for (placement_id, start_sample) in placements {
        // The placement was deleted while the render ran — installing
        // audio for it would resurrect the section's vocal (VIEW-04).
        if r.compose.find_placement(placement_id).is_none() {
            continue;
        }
        if let Some((old_id, old_path)) = r
            .compose
            .vocal_audio
            .clips
            .remove(&(definition_id, placement_id, track_id))
        {
            let _ = r.engine
                .send(AudioCommand::DeleteClip { clip_id: old_id });
            unlink_if_exists(&old_path);
        }

        let audio_clip_id = r.compose.fresh_derived_clip_id();
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
    }
    true
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

/// Drop every previously-installed vocal audio clip on this (def, track)
/// pair from both the engine and disk. Run before the new audio is
/// installed so we don't leak WAV files.
pub(super) fn tear_down_old_vocal_audio(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) {
    use resonance_audio::types::{AudioCommand, ClipId};
    type VocalAudioKey = (u64, u64, TrackId);
    type VocalAudioEntry = (ClipId, std::path::PathBuf);
    let stale: Vec<(VocalAudioKey, VocalAudioEntry)> = r
        .compose
        .vocal_audio
        .clips
        .iter()
        .filter(|((d, _p, t), _)| *d == definition_id && *t == track_id)
        .map(|(k, v)| (*k, v.clone()))
        .collect();
    for (key, (clip_id, path)) in stale {
        let _ = r.engine.send(AudioCommand::DeleteClip { clip_id });
        unlink_if_exists(&path);
        r.compose.vocal_audio.clips.remove(&key);
    }
}
