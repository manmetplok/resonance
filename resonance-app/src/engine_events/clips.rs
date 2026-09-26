//! App-side handlers for audio clip events from the engine.

use resonance_audio::types::*;

use crate::state::ClipState;
use crate::Resonance;

pub(super) fn imported(
    r: &mut Resonance,
    clip_id: ClipId,
    track_id: TrackId,
    start_sample: SamplePos,
    duration_samples: u64,
    name: String,
    waveform_peaks: Vec<(f32, f32)>,
) {
    // The load of a clip whose deletion echo is still owed (ARCH-01
    // A-13i): the engine deletes it right after this (the delete waited
    // for the load to land), and the mirror already dropped it — or holds
    // the instance a later restore re-added, whose own load echoes after.
    if r.io.restore_echoes.clip_deletion_owed(clip_id) {
        return;
    }
    // A stale import — queued in a project that a load or slow-path undo
    // has since replaced — names a track this project doesn't have, or an
    // id that is another track's clip here. Drop it rather than overwrite
    // that clip's waveform or add a phantom clip (code review UPD-09; the
    // engine fences these too).
    let known_track = r.registry.tracks.iter().any(|t| t.id == track_id);
    let same_track = r
        .clips
        .iter()
        .find(|c| c.id == clip_id)
        .is_none_or(|c| c.track_id == track_id);
    if !known_track || !same_track {
        return;
    }
    // Idempotent: if the clip already exists (created by project load),
    // just update its waveform and total frames. Otherwise push new.
    if let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) {
        clip.waveform_peaks = waveform_peaks;
        clip.total_frames = duration_samples + clip.trim_start_frames + clip.trim_end_frames;
    } else {
        r.clips.push(ClipState {
            id: clip_id,
            track_id,
            start_sample,
            duration_samples,
            name,
            total_frames: duration_samples,
            trim_start_frames: 0,
            trim_end_frames: 0,
            fade_in_frames: 0,
            fade_in_curve: FadeCurve::default(),
            fade_out_frames: 0,
            fade_out_curve: FadeCurve::default(),
            gain_db: 0.0,
            waveform_peaks,
            vocal_tuning: None,
            // Engine `ClipImported` carries no asset id; the import /
            // placement orchestration (doc #175) sets this link app-side
            // once the clip exists. A recorded/bounced clip stays `None`.
            asset_ref: None,
        });
    }
}

/// The `ClipDeleted` echo: swallowed when a diff restore or a live delete
/// already mirrored it (ARCH-01 A-13i — the id may name a clip a later
/// restore re-added), mirrored otherwise.
pub(super) fn deleted_echo(r: &mut Resonance, clip_id: ClipId) {
    if r.io.restore_echoes.settle_clip_deleted(clip_id) {
        return;
    }
    deleted(r, clip_id);
}

/// Mirror an audio clip's deletion.
pub(crate) fn deleted(r: &mut Resonance, clip_id: ClipId) {
    r.clips.retain(|c| c.id != clip_id);
    // The Pool "used ×N" badge / `pool.list` count (review VIEW-30).
    r.recompute_pool_usage();
    // Drop any vocal-audio-clip side-table entries that reference
    // this clip. Without this, an engine-side delete would leave a
    // dangling `(ClipId, PathBuf)` in `vocal_audio.clips` that the
    // next regen's `tear_down_old_vocal_audio` would try to re-delete
    // (engine returns "unknown clip"; unlink fails on the already-
    // removed WAV) and then never clear, since the entry is keyed by
    // (def, placement, track) — not by clip id.
    r.compose.vocal_audio.forget_deleted_clip(clip_id);
}

pub(super) fn moved(
    r: &mut Resonance,
    clip_id: ClipId,
    new_start_sample: SamplePos,
    new_track_id: TrackId,
) {
    // An edit of the instance whose deletion is still owed (A-13i).
    if r.io.restore_echoes.clip_deletion_owed(clip_id) {
        return;
    }
    if let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.track_id = new_track_id;
    }
}

pub(super) fn trimmed(
    r: &mut Resonance,
    clip_id: ClipId,
    new_start_sample: SamplePos,
    new_duration_samples: u64,
    trim_start_frames: u64,
    trim_end_frames: u64,
) {
    // An edit of the instance whose deletion is still owed (A-13i).
    if r.io.restore_echoes.clip_deletion_owed(clip_id) {
        return;
    }
    if let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.duration_samples = new_duration_samples;
        clip.trim_start_frames = trim_start_frames;
        clip.trim_end_frames = trim_end_frames;
    }
}

pub(super) fn fade_changed(
    r: &mut Resonance,
    clip_id: ClipId,
    fade_in_frames: u64,
    fade_in_curve: FadeCurve,
    fade_out_frames: u64,
    fade_out_curve: FadeCurve,
) {
    // An edit of the instance whose deletion is still owed (A-13i).
    if r.io.restore_echoes.clip_deletion_owed(clip_id) {
        return;
    }
    if let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) {
        clip.fade_in_frames = fade_in_frames;
        clip.fade_in_curve = fade_in_curve;
        clip.fade_out_frames = fade_out_frames;
        clip.fade_out_curve = fade_out_curve;
    }
}

pub(super) fn gain_changed(r: &mut Resonance, clip_id: ClipId, gain_db: f32) {
    // An edit of the instance whose deletion is still owed (A-13i).
    if r.io.restore_echoes.clip_deletion_owed(clip_id) {
        return;
    }
    if let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) {
        clip.gain_db = gain_db;
    }
}

pub(super) fn recording_finished(
    r: &mut Resonance,
    clip_id: ClipId,
    track_id: TrackId,
    start_sample: SamplePos,
    duration_samples: u64,
    name: String,
    waveform_peaks: Vec<(f32, f32)>,
) {
    // A take is an undoable edit; snapshot before it lands (STATE-02).
    r.record_recording_edit();
    r.clips.push(ClipState {
        id: clip_id,
        track_id,
        start_sample,
        duration_samples,
        name,
        total_frames: duration_samples,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks,
        vocal_tuning: None,
        // A freshly recorded clip isn't a pool import.
        asset_ref: None,
    });
    r.transport.recording = false;

    // Auto-switch to Recorded playback after a take lands on an
    // external-instrument track (doc #257, todo #1100): the user just
    // captured the hardware — playback should now play the take instead
    // of re-driving the synth over it. External tracks have no
    // track-type discriminant; presence in the `external_instruments`
    // map is the marker (cf. todo #457). Engine-owned, non-undoable
    // toggle like monitor/arm: mirror optimistically and dispatch; the
    // engine echoes `TrackPlaybackSourceChanged`.
    if r.devices.external_instruments.contains_key(&track_id) {
        let source = resonance_common::PlaybackSource::Recorded;
        if let Some(track) = r.registry.tracks.iter_mut().find(|t| t.id == track_id) {
            track.playback_source = source;
        }
        let _ = r
            .engine
            .send(AudioCommand::SetTrackPlaybackSource { track_id, source });
    }
}

/// Mirror a finished vocal pitch analysis (`AudioEvent::ClipPitchDetected`,
/// todo #357) into the matching clip's GUI-side [`ClipState::vocal_tuning`].
///
/// The detected `contour` and `notes` replace whatever the previous
/// analysis stored, exactly as the engine replaced its own cache. The
/// global key / scale / correction parameters are app-side user settings
/// that analysis never derives, so they are preserved across re-analysis
/// by inserting into the existing model rather than overwriting it. A
/// no-op when no clip matches `clip_id` (e.g. the clip was deleted while
/// analysis was running off-thread).
pub(super) fn pitch_detected(
    r: &mut Resonance,
    clip_id: ClipId,
    notes: Vec<NoteBlob>,
    contour: Vec<F0Frame>,
) {
    if let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) {
        let tuning = clip.vocal_tuning.get_or_insert_with(VocalTuning::default);
        tuning.contour = contour;
        tuning.notes = notes;
    }
}

/// Send `DeleteClip` for a clip the caller drops from the mirror itself,
/// right now (STATE-10), and owe its `ClipDeleted` echo (ARCH-01 A-13i):
/// an undo may re-add the clip under this id before the echo lands. Call
/// it while the clip is still mirrored — only a mirrored clip is owed, as
/// the engine answers a delete of an id it never loaded with no echo.
pub(crate) fn send_mirrored_delete(r: &mut Resonance, clip_id: ClipId) {
    let _ = r.engine.send(AudioCommand::DeleteClip { clip_id });
    if r.clips.iter().any(|c| c.id == clip_id) {
        r.io.restore_echoes.expect_clip_deleted(clip_id);
    }
}
