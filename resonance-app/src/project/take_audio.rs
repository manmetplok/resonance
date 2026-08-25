//! Reading a cycle-record take's recording back out of the project
//! bundle (epic #15, ba todo #1400).
//!
//! A take names its recording by `clip_ref` and nothing else: the WAV is
//! at [`clip_audio_file`](super::clip_audio_file) inside the project
//! directory, streamed there by `recording.rs` as the pass was captured
//! and finalized at the loop seam, *before* `TakeCaptured` is emitted.
//! That is the only durable account of the audio the app has — a take
//! clip never enters [`Resonance::clips`](crate::Resonance) (ba todo
//! #1396), so there is no mirrored clip to borrow peaks from.
//!
//! Hence this module: the take lane derives its waveform from the file,
//! app-side, on **both** paths that can produce a take.
//!
//! # Why derive rather than be told
//!
//! The engine already computes these peaks while recording
//! ([`RolledAudioTake::waveform_peaks`](resonance_audio::RolledAudioTake))
//! and could carry them on `TakeCaptured`. That would remove the capture
//! path's read — but not this module, because it would do nothing for a
//! **project load**: peaks are not persisted (deliberately; a project
//! with twenty passes would carry megabytes of redundant min/max pairs,
//! which is why [`PoolAsset`](crate::state::pool::PoolAsset) drops its
//! thumbnails on save and rebuilds them too). A reloaded take would then
//! have peaks after capture and none after reopening the project — the
//! exact after-capture-only asymmetry todo #1396 rejected when it put
//! `extent` on the model instead of on the event.
//!
//! So a file-derived answer has to exist for the load path regardless,
//! and once it exists the capture path using it too is what makes the
//! two paths identical by construction. Delivering the peaks on the
//! event as well would be an optimisation of the capture path, not a
//! different design; it is filed as a follow-up rather than done here
//! because it is a `resonance-audio` change.
//!
//! # Bucketing
//!
//! [`compute_waveform_peaks`] — the engine's own function, over the same
//! `WAVEFORM_PEAK_FRAMES` buckets a placed clip uses. That is not
//! incidental: the take lane indexes peaks by *clip frame* (doc #293), so
//! the bucket size has to be the one the indexing assumes, and the same
//! audio dropped on a track draws the identical silhouette.

use std::path::Path;

use resonance_audio::types::{compute_waveform_peaks, ClipSource};
use resonance_common::ClipId;

/// Read the recording a take's `clip_ref` names and reduce it to waveform
/// peaks, or say why it could not be read.
///
/// `project_dir` is the `.rproj` directory; the WAV is resolved through
/// [`clip_audio_file`](super::clip_audio_file), the one definition of
/// that path.
///
/// **An error means "this machine cannot show you this recording"** — the
/// file is gone, is not the engine's 32-bit-float format, or is
/// truncated. All three are the same fact to the user and to the engine,
/// which memory-maps the very same file to play the take, so they are all
/// the missing-media state rather than three shades of it. Callers decide
/// what to do about it: a project load flags the take (todo #412), a
/// fresh capture does not — see [`crate::engine_events::takes`].
pub fn load_take_peaks(
    project_dir: &Path,
    clip_ref: ClipId,
) -> Result<Vec<(f32, f32)>, String> {
    let path = project_dir.join(super::clip_audio_file(clip_ref));
    let source = ClipSource::open_wav(&path)?;
    let peaks = compute_waveform_peaks(source.as_frames());
    if peaks.is_empty() {
        return Err(format!("{} holds no audio frames", path.display()));
    }
    Ok(peaks)
}
