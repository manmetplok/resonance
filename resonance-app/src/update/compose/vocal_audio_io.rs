//! Filesystem side of the vocal render pipeline: where rendered WAVs
//! land, what they're called, and how superseded ones are reaped.
//!
//! Split out of `vocal_render` (ba todo #1259). The update handler was
//! the only one in `update/` doing raw `std::fs` work — directory
//! layout, `remove_file`, WAV writing — mixed in with message handling.
//! Everything that touches the disk now lives here, so the handler is
//! left deciding *what* to render, never *where the bytes land*.
//!
//! Nothing in this module reads [`crate::Resonance`]: the destination
//! directory is derived from a plain project path and the render job is
//! a self-contained, `Send` bundle that runs on a blocking thread.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::g2p::AssignedSyllable;
use resonance_music_theory::VocalParams;

use crate::compose::vocal_svs::{self, SvsRenderCache};
use crate::compose::ExpressionCurves;

/// Destination directory for rendered vocal WAVs. Prefers the loaded
/// project's `audio/` subdirectory so saves capture the clip; falls
/// back to a per-process temp dir for unsaved sessions.
///
/// `project_path` is the project *file* (`…/song.rson`), so the audio
/// directory is its parent's `audio/`.
pub fn vocal_audio_dir(project_path: Option<&Path>) -> PathBuf {
    project_path
        .and_then(|p| p.parent().map(|d| d.join("audio")))
        .unwrap_or_else(|| std::env::temp_dir().join("resonance_vocal"))
}

/// Best-effort file delete. Missing files (e.g. a previous render
/// failed to write or was already cleaned up) are silently ignored;
/// any other error is surfaced via stderr but does not fail the regen.
///
/// On Linux it's safe to `unlink` a file the engine still has mmap'd —
/// the kernel keeps the inode alive until the mapping is dropped and
/// reclaims the disk space then.
pub fn unlink_if_exists(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("[vocal] unlink {}: {e}", path.display()),
    }
}

/// Filename for one rendered take. Timestamped rather than derived from
/// the lane so a fresh render never overwrites the WAV the engine may
/// still be playing from — the superseded file is unlinked separately,
/// after the new clip is installed.
pub fn render_wav_filename() -> String {
    format!(
        "vocal_{}.wav",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    )
}

/// Write one rendered take into `dest_dir`, returning the file it
/// landed in.
pub fn write_rendered_wav(
    dest_dir: &Path,
    samples_stereo: &[f32],
    sample_rate: u32,
) -> Result<PathBuf, String> {
    let path = dest_dir.join(render_wav_filename());
    vocal_svs::write_stereo_wav(&path, samples_stereo, sample_rate)
        .map_err(|e| format!("write WAV {}: {e}", path.display()))?;
    Ok(path)
}

/// One off-thread vocal render: everything the SVS pipeline needs,
/// owned, so the bundle can be moved onto a blocking thread.
///
/// `render_cache` is the lane's shared content-addressed cache — the
/// same `Arc` the update handler keeps in `compose.vocal_audio`, so the
/// segment-level reuse decisions and the `last_plan` tally the "N of M
/// segments changed" overlay reads survive the round trip (#495).
pub(crate) struct VocalRenderJob {
    pub midi_notes: Vec<MidiNote>,
    pub params: VocalParams,
    pub assigned: Vec<AssignedSyllable>,
    pub curves: ExpressionCurves,
    pub bpm: f32,
    pub engine_sample_rate: u32,
    pub dest_dir: PathBuf,
    pub render_cache: Arc<Mutex<SvsRenderCache>>,
}

impl VocalRenderJob {
    /// Run the SVS pipeline and write the WAV. Blocking.
    ///
    /// Returns `Ok(None)` when the SVS model dir isn't installed (silent
    /// fallback to MIDI-only mode), `Ok(Some((path, trim_start_frames,
    /// trim_end_frames)))` on success.
    pub(crate) fn run(&self) -> Result<Option<(PathBuf, u64, u64)>, String> {
        let mut cache = self
            .render_cache
            .lock()
            .map_err(|_| "vocal render cache poisoned".to_string())?;
        let rendered = match vocal_svs::render_vocal_clip(
            &self.midi_notes,
            &self.params,
            &self.assigned,
            &self.curves,
            TICKS_PER_QUARTER_NOTE as u32,
            self.bpm,
            self.engine_sample_rate,
            &mut cache,
        ) {
            Ok(Some(r)) => r,
            Ok(None) => return Ok(None),
            Err(e) => return Err(format!("SVS render: {e}")),
        };

        let path = write_rendered_wav(
            &self.dest_dir,
            &rendered.samples_stereo,
            rendered.sample_rate,
        )?;
        Ok(Some((
            path,
            rendered.trim_start_frames,
            rendered.trim_end_frames,
        )))
    }
}
