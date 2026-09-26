//! File-system I/O for Resonance projects: save, load, autosave,
//! crash-safe atomic writes, and the versioned backup subsystem.
//!
//! All serde structs and format constants live in [`super::model`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use resonance_audio::midi_io;
use resonance_audio::types::{ClipId, MidiNote, PluginInstanceId};

use super::model::{
    LoadedProject, ProjectFile, ProjectMidiNote, ProjectPlugin, AUTOSAVE_JSON,
    PROJECT_FORMAT_VERSION, PROJECT_JSON,
};

/// Write a project to disk. Assumes the engine has already written
/// every audio clip's WAV file into `{path}/audio/`; this function
/// only writes `project.json`, the MIDI clip files, and the plugin
/// state blobs.
pub fn save_project(
    path: &Path,
    project: &ProjectFile,
    plugin_states: &[(PluginInstanceId, Vec<u8>)],
    midi_clips: &[(ClipId, Vec<MidiNote>)],
) -> Result<(), String> {
    write_project_metadata(path, PROJECT_JSON, "", project, plugin_states, midi_clips)
}

/// Project-relative subtree an autosave writes its MIDI and plugin files
/// into, so it never touches the ones `project.json` points to.
pub const AUTOSAVE_SIDECAR_DIR: &str = "autosave";

/// Write an autosave snapshot. The project metadata goes to
/// [`AUTOSAVE_JSON`] instead of [`PROJECT_JSON`], and the MIDI files and
/// plugin blobs go under [`AUTOSAVE_SIDECAR_DIR`] with the snapshot's
/// JSON pointing there — so neither the canonical `project.json` nor the
/// files it references are ever overwritten (code review STATE-11: a
/// "Don't save" quit used to reopen the old arrangement with the
/// autosave's notes and plugin states). Audio WAVs stay shared: a clip's
/// WAV never changes once written.
pub fn save_autosave(
    path: &Path,
    project: &ProjectFile,
    plugin_states: &[(PluginInstanceId, Vec<u8>)],
    midi_clips: &[(ClipId, Vec<MidiNote>)],
) -> Result<(), String> {
    write_project_metadata(
        path,
        AUTOSAVE_JSON,
        AUTOSAVE_SIDECAR_DIR,
        project,
        plugin_states,
        midi_clips,
    )
}

/// Shared body of [`save_project`] / [`save_autosave`]: write the plugin
/// state blobs and MIDI clip files under `{path}/{sidecar_root}`, then the
/// project-metadata JSON under `json_file_name`, its `state_file` /
/// `midi_file` paths pointing at those files. Every write goes through
/// [`atomic_write`].
fn write_project_metadata(
    path: &Path,
    json_file_name: &str,
    sidecar_root: &str,
    project: &ProjectFile,
    plugin_states: &[(PluginInstanceId, Vec<u8>)],
    midi_clips: &[(ClipId, Vec<MidiNote>)],
) -> Result<(), String> {
    let rel = |sub: &str| {
        if sidecar_root.is_empty() {
            sub.to_string()
        } else {
            format!("{sidecar_root}/{sub}")
        }
    };
    let plugins_dir = path.join(rel("plugins"));
    let midi_dir = path.join(rel("midi"));
    std::fs::create_dir_all(&plugins_dir).map_err(|e| format!("Create plugins dir: {e}"))?;
    std::fs::create_dir_all(&midi_dir).map_err(|e| format!("Create midi dir: {e}"))?;

    // Write plugin state blobs.
    for (instance_id, data) in plugin_states {
        let file_name = format!("plugin_{instance_id}.bin");
        let file_path = plugins_dir.join(&file_name);
        atomic_write(&file_path, data).map_err(|e| format!("Write {file_name}: {e}"))?;
    }

    // Write MIDI clips as Standard MIDI Files.
    for (clip_id, notes) in midi_clips {
        let file_path = midi_dir.join(format!("clip_{clip_id}.mid"));
        let bytes = midi_io::encode_midi(notes).map_err(|e| format!("Encode midi {clip_id}: {e}"))?;
        atomic_write(&file_path, &bytes).map_err(|e| format!("Write midi {clip_id}: {e}"))?;
    }

    // Write the project-metadata JSON (project.json or, for an autosave,
    // project.autosave.json). Each MIDI clip carries its notes inline:
    // that copy is lossless, the `.mid` above is not (code review
    // STATE-06).
    let notes_by_clip: HashMap<ClipId, &Vec<MidiNote>> =
        midi_clips.iter().map(|(id, notes)| (*id, notes)).collect();
    let mut project = project.clone();
    if !sidecar_root.is_empty() {
        for mc in &mut project.midi_clips {
            mc.midi_file = rel(&format!("midi/clip_{}.mid", mc.id));
        }
        let plugins = project
            .tracks
            .iter_mut()
            .flat_map(|t| t.plugins.iter_mut())
            .chain(project.busses.iter_mut().flat_map(|b| b.plugins.iter_mut()))
            .chain(project.master_plugins.iter_mut());
        for plugin in plugins {
            plugin.state_file = rel(&format!("plugins/plugin_{}.bin", plugin.instance_id));
        }
    }
    for mc in &mut project.midi_clips {
        if let Some(notes) = notes_by_clip.get(&mc.id) {
            mc.notes = Some(
                notes
                    .iter()
                    .map(|n| ProjectMidiNote {
                        note: n.note,
                        velocity: n.velocity,
                        start_tick: n.start_tick,
                        duration_ticks: n.duration_ticks,
                    })
                    .collect(),
            );
        }
    }
    let json =
        serde_json::to_string_pretty(&project).map_err(|e| format!("Serialize project: {e}"))?;
    atomic_write(&path.join(json_file_name), json.as_bytes())
        .map_err(|e| format!("Write {json_file_name}: {e}"))?;

    Ok(())
}

/// Crash-safe file write, shared with every other user-state store: a
/// unique temp name per write, and a failed write never strands its temp
/// file. The project writer used to carry its own copy with a fixed
/// `<name>.tmp` (code review FU-M6a).
pub use resonance_common::atomic_write;

/// One timestamped snapshot under a project's `backups/` directory.
/// Returned by [`list_backups`] for the restore UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupEntry {
    /// Absolute path to the backup file (`backups/project-<rfc3339>.json`).
    pub path: PathBuf,
    /// The RFC3339 UTC timestamp embedded in the file name, verbatim.
    pub timestamp: String,
}

/// Current wall clock as an RFC3339 UTC string suitable for a backup
/// file name. Callers pass the result to [`write_backup`]; keeping the
/// clock read out of `write_backup` lets tests drive rotation with
/// deterministic timestamps.
pub fn backup_timestamp_now() -> String {
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;

    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        // Formatting RFC3339 from a valid `OffsetDateTime` is infallible
        // in practice; fall back to an epoch-seconds stamp rather than
        // unwrap so a backup is still written under some sortable name.
        .unwrap_or_else(|_| {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("epoch-{secs}")
        })
}

fn backup_file_name(timestamp: &str) -> String {
    format!("project-{timestamp}.json")
}

/// Snapshot the project's already-written `project.json` into
/// `backups/project-<timestamp>.json` (atomic write) and prune the
/// oldest snapshots so at most `retention` remain.
///
/// Call this only after a successful save: it reads the canonical
/// `project.json`, so a failed save (which never wrote it, or left the
/// prior copy intact) is never captured as a fresh backup. Audio WAVs
/// are shared (a clip's WAV never changes); the MIDI files and plugin
/// blobs, which every save rewrites, are hard-linked into
/// `backups/project-<timestamp>.files/` and the snapshot points there, so
/// a backup restores its own versions (code review STATE-12). Pruning a
/// snapshot removes its files dir too. [`load_project`] opens a backup
/// file directly, resolving it against the bundle root.
///
/// `timestamp` is the RFC3339 UTC stamp for the file name (see
/// [`backup_timestamp_now`]). A `retention` of 0 prunes every snapshot,
/// including the one just written; callers that want backups pass `>= 1`.
pub fn write_backup(project_dir: &Path, timestamp: &str, retention: u32) -> Result<PathBuf, String> {
    let source = project_dir.join("project.json");
    let mut bytes =
        std::fs::read(&source).map_err(|e| format!("Read project.json for backup: {e}"))?;

    let backups_dir = project_dir.join("backups");
    std::fs::create_dir_all(&backups_dir).map_err(|e| format!("Create backups dir: {e}"))?;

    // Freeze the MIDI files and plugin blobs the snapshot references: a
    // later save rewrites them in place (code review STATE-12). Anything
    // that isn't a project JSON has nothing to freeze and is copied as is.
    if let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        let files_rel = format!("backups/{}", backup_files_dir_name(timestamp));
        freeze_backup_side_files(&mut json, project_dir, &files_rel)?;
        bytes = serde_json::to_vec_pretty(&json).map_err(|e| format!("Serialize backup: {e}"))?;
    }

    let dest = backups_dir.join(backup_file_name(timestamp));
    atomic_write(&dest, &bytes).map_err(|e| format!("Write backup: {e}"))?;

    prune_backups(&backups_dir, retention)?;
    Ok(dest)
}

/// The directory beside `backups/project-<timestamp>.json` holding the
/// side files that snapshot references.
fn backup_files_dir_name(timestamp: &str) -> String {
    format!("project-{timestamp}.files")
}

/// Hard-link (copy where linking fails, e.g. across filesystems) every
/// plugin blob and MIDI file `json` references into `{files_rel}/` and
/// repoint the references there. Saves replace those files by rename
/// ([`atomic_write`]), so a hard link keeps the snapshot's version for
/// free. A reference whose file is missing is left as it was.
fn freeze_backup_side_files(
    json: &mut serde_json::Value,
    project_dir: &Path,
    files_rel: &str,
) -> Result<(), String> {
    let mut refs: Vec<&mut serde_json::Value> = Vec::new();
    let obj = match json.as_object_mut() {
        Some(o) => o,
        None => return Ok(()),
    };
    for (key, value) in obj.iter_mut() {
        let (field, nested) = match key.as_str() {
            "tracks" | "busses" => ("state_file", true),
            "master_plugins" => ("state_file", false),
            "midi_clips" => ("midi_file", false),
            _ => continue,
        };
        let Some(items) = value.as_array_mut() else {
            continue;
        };
        for item in items {
            if nested {
                let Some(ps) = item.get_mut("plugins").and_then(|p| p.as_array_mut()) else {
                    continue;
                };
                refs.extend(ps.iter_mut().filter_map(|p| p.get_mut(field)));
            } else if let Some(f) = item.get_mut(field) {
                refs.push(f);
            }
        }
    }

    for r in refs {
        let Some(rel) = r.as_str().map(str::to_string) else {
            continue;
        };
        let src = project_dir.join(&rel);
        if rel.starts_with("backups/") || !src.is_file() {
            continue;
        }
        let dest = project_dir.join(files_rel).join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Create backup dir: {e}"))?;
        }
        let _ = std::fs::remove_file(&dest);
        if std::fs::hard_link(&src, &dest).is_err() {
            std::fs::copy(&src, &dest).map_err(|e| format!("Copy {rel} into backup: {e}"))?;
        }
        *r = serde_json::Value::String(format!("{files_rel}/{rel}"));
    }
    Ok(())
}

/// Delete the oldest snapshots in `backups_dir` (and the side files each
/// one owns) until at most `retention` remain. Newest-first ordering comes
/// from [`scan_backups`].
fn prune_backups(backups_dir: &Path, retention: u32) -> Result<(), String> {
    for entry in scan_backups(backups_dir).into_iter().skip(retention as usize) {
        std::fs::remove_file(&entry.path)
            .map_err(|e| format!("Prune backup {}: {e}", entry.path.display()))?;
        let files = backups_dir.join(backup_files_dir_name(&entry.timestamp));
        if files.exists() {
            std::fs::remove_dir_all(&files)
                .map_err(|e| format!("Prune backup files {}: {e}", files.display()))?;
        }
    }
    Ok(())
}

/// List the versioned backups under `{project_dir}/backups`, newest
/// first, for the restore UI. A missing or unreadable `backups/` dir
/// yields an empty list rather than an error — there's simply nothing to
/// restore.
pub fn list_backups(project_dir: &Path) -> Vec<BackupEntry> {
    scan_backups(&project_dir.join("backups"))
}

/// Scan a `backups/` directory for `project-<timestamp>.json` snapshots,
/// sorted newest-first by their embedded RFC3339 timestamp. Leftover
/// `*.tmp` files from an interrupted [`atomic_write`] are ignored (they
/// don't end in `.json`), as is any unrelated file.
fn scan_backups(backups_dir: &Path) -> Vec<BackupEntry> {
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;

    let read_dir = match std::fs::read_dir(backups_dir) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };

    let mut entries: Vec<(Option<OffsetDateTime>, BackupEntry)> = read_dir
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_name()?.to_str()?;
            let timestamp = name.strip_prefix("project-")?.strip_suffix(".json")?;
            let parsed = OffsetDateTime::parse(timestamp, &Rfc3339).ok();
            Some((
                parsed,
                BackupEntry {
                    path: path.clone(),
                    timestamp: timestamp.to_string(),
                },
            ))
        })
        .collect();

    // Newest first. Parse the RFC3339 stamp rather than string-compare:
    // variable sub-second precision (`…00Z` vs `…00.5Z`) doesn't sort
    // chronologically as plain text. Unparseable names sort oldest.
    entries.sort_by(|(a, _), (b, _)| match (a, b) {
        (Some(a), Some(b)) => b.cmp(a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    entries.into_iter().map(|(_, e)| e).collect()
}

/// Read a project from disk. Audio clips stay on disk and are
/// memory-mapped by the engine's load-clip handler — this function
/// only parses `project.json`, loads MIDI clips from their `.mid`
/// files, and collects plugin state blobs.
pub fn load_project(path: &Path) -> Result<LoadedProject, String> {
    let json_path = if path.join("project.json").exists() {
        path.join("project.json")
    } else if path.is_file() && path.extension().is_some_and(|e| e == "json") {
        // `project.json`, `project.autosave.json` or a versioned backup
        // (`backups/project-<timestamp>.json`, code review STATE-12).
        path.to_path_buf()
    } else {
        return Err("No project.json found".to_string());
    };

    // A backup's references are relative to the bundle root, one level
    // above `backups/`.
    let parent = json_path.parent().map(|p| p.to_path_buf());
    let project_dir = match parent {
        Some(p) if p.file_name().is_some_and(|n| n == "backups") => {
            p.parent().map(|g| g.to_path_buf()).unwrap_or(p)
        }
        Some(p) => p,
        None => path.to_path_buf(),
    };

    let json =
        std::fs::read_to_string(&json_path).map_err(|e| format!("Read project.json: {e}"))?;
    let file: ProjectFile =
        serde_json::from_str(&json).map_err(|e| format!("Parse project.json: {e}"))?;

    if file.version > PROJECT_FORMAT_VERSION {
        return Err(format!(
            "Project version {} is newer than this build (v{}). \
             Please update Resonance.",
            file.version, PROJECT_FORMAT_VERSION
        ));
    }

    // Read MIDI clip files. Missing files are logged and replaced
    // with empty note lists so the rest of the project still loads.
    let mut midi_notes: HashMap<ClipId, Vec<MidiNote>> = HashMap::new();
    for mc in &file.midi_clips {
        // The inline copy is lossless (code review STATE-06); the `.mid`
        // is the fallback for projects saved before it existed.
        if let Some(notes) = &mc.notes {
            let notes = notes
                .iter()
                .map(|n| MidiNote {
                    note: n.note,
                    velocity: n.velocity,
                    start_tick: n.start_tick,
                    duration_ticks: n.duration_ticks,
                })
                .collect();
            midi_notes.insert(mc.id, notes);
            continue;
        }
        let mid_path = project_dir.join(&mc.midi_file);
        match midi_io::read_midi_file(&mid_path) {
            Ok(notes) => {
                midi_notes.insert(mc.id, notes);
            }
            Err(e) => {
                eprintln!("Warning: could not load midi file {}: {e}", mc.midi_file);
                midi_notes.insert(mc.id, Vec::new());
            }
        }
    }

    // Read plugin state blobs for every track, bus, and master plugin.
    let mut plugin_states: HashMap<PluginInstanceId, std::sync::Arc<[u8]>> = HashMap::new();
    let mut load_plugin_state = |plugin: &ProjectPlugin| {
        let state_path = project_dir.join(&plugin.state_file);
        match std::fs::read(&state_path) {
            Ok(data) => {
                plugin_states.insert(plugin.instance_id, data.into());
            }
            Err(e) => {
                eprintln!(
                    "Warning: could not load plugin state {}: {e}",
                    plugin.state_file
                );
            }
        }
    };
    for track in &file.tracks {
        for plugin in &track.plugins {
            load_plugin_state(plugin);
        }
    }
    for bus in &file.busses {
        for plugin in &bus.plugins {
            load_plugin_state(plugin);
        }
    }
    for plugin in &file.master_plugins {
        load_plugin_state(plugin);
    }

    Ok(LoadedProject {
        file,
        project_dir,
        midi_notes,
        plugin_states,
    })
}
