//! Save As: pointing the open project at a new bundle and saving it there.

use iced::Task;

use crate::message::Message;
use crate::Resonance;

/// `path` as a project bundle path: `.rproj` appended unless it already
/// ends in it. Case-insensitive: `Song.RPROJ` / `Song.Rproj` already carry
/// the extension and must not be doubled into `Song.RPROJ.rproj`. Callers
/// (the control `project.save` / `save_as` handlers included) may already
/// have normalized this, but it is repeated here so the save handlers stay
/// correct standalone for their other caller, the rfd save dialog.
pub(super) fn with_rproj_extension(path: String) -> std::path::PathBuf {
    let has_rproj_ext = std::path::Path::new(&path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("rproj"));
    if has_rproj_ext {
        std::path::PathBuf::from(path)
    } else {
        std::path::PathBuf::from(format!("{path}.rproj"))
    }
}

/// Point the project at `path` and save it there.
pub(super) fn save_as_to(r: &mut Resonance, path: std::path::PathBuf) -> Task<Message> {
    // The session writes clip WAVs into this bundle from now on; one it
    // already holds must not get its clip ids re-issued (FU-A6c), nor its
    // asset ids (D-7a).
    r.seed_clip_ids_on_disk(&path);
    r.seed_asset_ids_on_disk(&path);
    // The undo history names clips by `audio/clip_<id>.wav` under whatever
    // bundle is current, but the engine writes only the live clips into the
    // new one: the save carries the rest over from the old bundle (BUG-60).
    // A source a failed save left pending is older and is kept.
    if let Some(old) = r.io.project_path.take() {
        if !crate::project::clip_gc::same_dir(&old, &path) {
            r.io.carry_clip_wavs_from.get_or_insert(old);
        }
    }
    r.io.project_path = Some(path);
    super::start_save(r)
}
