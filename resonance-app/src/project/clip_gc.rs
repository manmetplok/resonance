//! Save-time collection of unreferenced `audio/clip_<id>.wav` files
//! (code review FU-V5a).
//!
//! A clip's WAV is written once and never rewritten, and nothing deleted
//! it: a removed clip, a superseded vocal render, a deleted take all left
//! theirs behind for the life of the bundle. After a successful manual
//! save [`reap_unreferenced_clip_wavs`] removes the ones nothing can name
//! any more. Conservative by construction — when in doubt, keep:
//!
//! - a file is kept when its id is named by the saved project, any undo
//!   or redo snapshot, the engine's clip list at save time (the caller's
//!   `keep`), or by any project JSON in the bundle root
//!   (`project.autosave.json`) or in `backups/` (a backup shares the
//!   bundle's audio by name, never by link — STATE-12);
//! - a file whose id is above every id so named is kept: ids are never
//!   reused (STATE-08), so it belongs to a clip created after the keep
//!   set was taken (a recording, a load still in flight);
//! - one JSON that cannot be read or parsed aborts the whole pass;
//! - only direct children of `audio/` named exactly `clip_<digits>.wav`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use resonance_audio::types::ClipId;
use resonance_common::TakeContent;

use super::ProjectFile;

/// Add every audio clip id `file` names — its clips and its audio takes'
/// recordings — to `out`.
pub fn collect_clip_ids(file: &ProjectFile, out: &mut BTreeSet<ClipId>) {
    out.extend(file.clips.iter().map(|c| c.id));
    for group in &file.take_groups {
        for take in &group.takes {
            if let TakeContent::Audio { clip_ref } = take.content {
                out.insert(clip_ref);
            }
        }
    }
}

/// The id in a `clip_<id>.wav` file name (or a path ending in one).
fn clip_wav_id(name: &str) -> Option<ClipId> {
    let file = name.rsplit(['/', '\\']).next()?;
    let digits = file.strip_prefix("clip_")?.strip_suffix(".wav")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Every clip id a project JSON on disk names, whatever its shape: any
/// string naming a `clip_<id>.wav`, and any `clip_ref` number.
fn collect_json_clip_ids(value: &serde_json::Value, out: &mut BTreeSet<ClipId>) {
    match value {
        serde_json::Value::String(s) => out.extend(clip_wav_id(s)),
        serde_json::Value::Array(items) => {
            items.iter().for_each(|v| collect_json_clip_ids(v, out))
        }
        serde_json::Value::Object(map) => {
            for (key, v) in map {
                if key == "clip_ref" {
                    out.extend(v.as_u64());
                }
                collect_json_clip_ids(v, out);
            }
        }
        _ => {}
    }
}

/// Add the clip ids of every `*.json` in `dir` (not recursive).
fn collect_dir_json_ids(dir: &Path, out: &mut BTreeSet<ClipId>) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("read {}: {e}", dir.display())),
    };
    for entry in entries {
        let path = entry.map_err(|e| format!("read {}: {e}", dir.display()))?.path();
        if !path.is_file() || path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let json: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| format!("parse {}: {e}", path.display()))?;
        collect_json_clip_ids(&json, out);
    }
    Ok(())
}

/// Delete `project_dir/audio/clip_<id>.wav` files no reference names; see
/// the module docs for what counts. `keep` is what the app knows (saved
/// project, undo/redo snapshots, engine clips); the bundle's own JSONs
/// are read here. Returns the removed paths, or why nothing was removed.
pub fn reap_unreferenced_clip_wavs(
    project_dir: &Path,
    keep: &BTreeSet<ClipId>,
) -> Result<Vec<PathBuf>, String> {
    let mut keep = keep.clone();
    collect_dir_json_ids(project_dir, &mut keep)?;
    collect_dir_json_ids(&project_dir.join("backups"), &mut keep)?;
    let Some(&newest) = keep.last() else {
        // Nothing names any clip: too little to go on — keep everything.
        return Ok(Vec::new());
    };

    let audio_dir = project_dir.join("audio");
    let entries = match std::fs::read_dir(&audio_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("read {}: {e}", audio_dir.display())),
    };
    let mut removed = Vec::new();
    for path in entries.filter_map(|e| e.ok()).map(|e| e.path()) {
        let Some(id) = path.file_name().and_then(|n| n.to_str()).and_then(clip_wav_id) else {
            continue;
        };
        if id > newest || keep.contains(&id) || !path.is_file() {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => removed.push(path),
            Err(e) => tracing::warn!(path = %path.display(), "[save] clip WAV GC: {e}"),
        }
    }
    Ok(removed)
}
