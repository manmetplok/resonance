//! File-system I/O for Resonance projects: save, load, autosave,
//! crash-safe atomic writes, and the versioned backup subsystem.
//!
//! All serde structs and format constants live in [`super::model`].

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use resonance_audio::midi_io;
use resonance_audio::types::{ClipId, MidiNote, PluginInstanceId};

use super::model::{
    LoadedProject, ProjectFile, ProjectPlugin, AUTOSAVE_JSON, PROJECT_FORMAT_VERSION, PROJECT_JSON,
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
    write_project_metadata(path, PROJECT_JSON, project, plugin_states, midi_clips)
}

/// Write an autosave snapshot. Identical to [`save_project`] except the
/// project metadata goes to [`AUTOSAVE_JSON`] instead of [`PROJECT_JSON`],
/// so the canonical `project.json` is never overwritten. The shared
/// audio / MIDI / plugin blobs are written to the same id-keyed paths as
/// a normal save, so the side file plus those blobs form a complete,
/// loadable snapshot for crash recovery.
pub fn save_autosave(
    path: &Path,
    project: &ProjectFile,
    plugin_states: &[(PluginInstanceId, Vec<u8>)],
    midi_clips: &[(ClipId, Vec<MidiNote>)],
) -> Result<(), String> {
    write_project_metadata(path, AUTOSAVE_JSON, project, plugin_states, midi_clips)
}

/// Shared body of [`save_project`] / [`save_autosave`]: write the plugin
/// state blobs and MIDI clip files, then the project-metadata JSON under
/// `json_file_name`. Every write goes through [`atomic_write`].
fn write_project_metadata(
    path: &Path,
    json_file_name: &str,
    project: &ProjectFile,
    plugin_states: &[(PluginInstanceId, Vec<u8>)],
    midi_clips: &[(ClipId, Vec<MidiNote>)],
) -> Result<(), String> {
    let plugins_dir = path.join("plugins");
    let midi_dir = path.join("midi");
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
    // project.autosave.json).
    let json =
        serde_json::to_string_pretty(project).map_err(|e| format!("Serialize project: {e}"))?;
    atomic_write(&path.join(json_file_name), json.as_bytes())
        .map_err(|e| format!("Write {json_file_name}: {e}"))?;

    Ok(())
}

/// Crash-safe file write: write `bytes` to a sibling `*.tmp` in the
/// same directory, fsync it, atomically rename it over `path`, then
/// fsync the parent directory so the rename itself reaches disk. A
/// crash at any point leaves either the previous file or the new file
/// fully intact — never a truncated target.
///
/// The temp file lives in the same directory as the target so the
/// rename stays within one filesystem (cross-device renames are not
/// atomic). Its name embeds the target's file name to avoid colliding
/// with the temp files of sibling writes in the same directory.
///
/// A leftover `*.tmp` from an interrupted write is inert: it shares no
/// name with any file the loader looks for, so it can never clobber a
/// good `project.json`, `.mid`, or `.bin`.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| format!("Atomic write target {} has no parent dir", path.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("Atomic write target {} has no file name", path.display()))?;

    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = parent.join(&tmp_name);

    // Write the full contents and fsync before the rename so the new
    // data is durable on disk before it becomes visible at `path`.
    {
        let mut f = std::fs::File::create(&tmp_path)
            .map_err(|e| format!("create {}: {e}", tmp_path.display()))?;
        f.write_all(bytes)
            .map_err(|e| format!("write {}: {e}", tmp_path.display()))?;
        f.sync_all()
            .map_err(|e| format!("fsync {}: {e}", tmp_path.display()))?;
    }

    // Atomic on POSIX: an observer sees either the old or the new file.
    std::fs::rename(&tmp_path, path).map_err(|e| {
        // Best-effort cleanup so a failed rename doesn't strand the tmp.
        let _ = std::fs::remove_file(&tmp_path);
        format!("rename {} -> {}: {e}", tmp_path.display(), path.display())
    })?;

    // fsync the directory so the rename entry itself survives a crash.
    // Directory fsync is unsupported on some platforms/filesystems, so
    // failures here are tolerated — the file data is already durable.
    if let Ok(dir) = std::fs::File::open(parent) {
        let _ = dir.sync_all();
    }

    Ok(())
}

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
/// prior copy intact) is never captured as a fresh backup. The
/// audio/MIDI/plugin blobs are *shared*, not copied — a backup is a
/// metadata snapshot whose relative paths still resolve against the
/// project directory.
///
/// `timestamp` is the RFC3339 UTC stamp for the file name (see
/// [`backup_timestamp_now`]). A `retention` of 0 prunes every snapshot,
/// including the one just written; callers that want backups pass `>= 1`.
pub fn write_backup(project_dir: &Path, timestamp: &str, retention: u32) -> Result<PathBuf, String> {
    let source = project_dir.join("project.json");
    let bytes =
        std::fs::read(&source).map_err(|e| format!("Read project.json for backup: {e}"))?;

    let backups_dir = project_dir.join("backups");
    std::fs::create_dir_all(&backups_dir).map_err(|e| format!("Create backups dir: {e}"))?;

    let dest = backups_dir.join(backup_file_name(timestamp));
    atomic_write(&dest, &bytes).map_err(|e| format!("Write backup: {e}"))?;

    prune_backups(&backups_dir, retention)?;
    Ok(dest)
}

/// Delete the oldest snapshots in `backups_dir` until at most `retention`
/// remain. Newest-first ordering comes from [`scan_backups`].
fn prune_backups(backups_dir: &Path, retention: u32) -> Result<(), String> {
    for entry in scan_backups(backups_dir).into_iter().skip(retention as usize) {
        std::fs::remove_file(&entry.path)
            .map_err(|e| format!("Prune backup {}: {e}", entry.path.display()))?;
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
    } else if path
        .file_name()
        .map(|f| f == "project.json")
        .unwrap_or(false)
    {
        path.to_path_buf()
    } else {
        return Err("No project.json found".to_string());
    };

    let project_dir = json_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| path.to_path_buf());

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
    let mut plugin_states: HashMap<PluginInstanceId, Vec<u8>> = HashMap::new();
    let mut load_plugin_state = |plugin: &ProjectPlugin| {
        let state_path = project_dir.join(&plugin.state_file);
        match std::fs::read(&state_path) {
            Ok(data) => {
                plugin_states.insert(plugin.instance_id, data);
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
