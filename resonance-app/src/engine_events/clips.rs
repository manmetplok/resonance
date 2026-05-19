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
            waveform_peaks,
        });
    }
}

pub(super) fn deleted(r: &mut Resonance, clip_id: ClipId) {
    r.clips.retain(|c| c.id != clip_id);
    // Drop any vocal-audio-clip side-table entries that reference
    // this clip. Without this, an engine-side delete would leave a
    // dangling `(ClipId, PathBuf)` in `vocal_audio.clips` that the
    // next regen's `tear_down_old_vocal_audio` would try to re-delete
    // (engine returns "unknown clip"; unlink fails on the already-
    // removed WAV) and then never clear, since the entry is keyed by
    // (def, placement, track) — not by clip id.
    r.compose
        .vocal_audio
        .clips
        .retain(|_, (id, _)| *id != clip_id);
}

pub(super) fn moved(
    r: &mut Resonance,
    clip_id: ClipId,
    new_start_sample: SamplePos,
    new_track_id: TrackId,
) {
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
    if let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) {
        clip.start_sample = new_start_sample;
        clip.duration_samples = new_duration_samples;
        clip.trim_start_frames = trim_start_frames;
        clip.trim_end_frames = trim_end_frames;
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
    r.clips.push(ClipState {
        id: clip_id,
        track_id,
        start_sample,
        duration_samples,
        name,
        total_frames: duration_samples,
        trim_start_frames: 0,
        trim_end_frames: 0,
        waveform_peaks,
    });
    r.transport.recording = false;
}
