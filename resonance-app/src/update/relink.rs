//! Update handlers for the missing-file relink flow (doc #175, todo
//! #600).
//!
//! When a project is loaded whose media pool references a WAV that is no
//! longer on disk, `restore_pool` keeps that asset — flagged
//! [`missing`](crate::state::pool::PoolAsset::missing) — so its clips stay
//! intact (offline). These handlers resolve the file again:
//!
//! * [`RelinkMessage::Locate`] opens an OS file picker for one asset; the
//!   chosen file comes back as [`RelinkMessage::Located`].
//! * [`RelinkMessage::SearchFolder`] opens a folder picker; the chosen
//!   folder ([`RelinkMessage::FolderChosen`]) is scanned recursively and
//!   *every* missing asset whose original filename is found is relinked in
//!   one shot.
//!
//! Resolving an asset copies/transcodes the source back into the project's
//! `audio/` folder under the asset's stable `asset_{id}.wav` name — reusing
//! the engine's import-to-pool path
//! ([`resonance_audio::import_one_to_pool`]) — on a worker thread. When it
//! finishes ([`RelinkMessage::Imported`]) the asset's missing flag is
//! cleared, its metadata refreshed, and every clip that references it is
//! reloaded via [`AudioCommand::LoadClipFromWav`] so playback resumes.
//!
//! Undo: the metadata change (missing → present, refreshed source
//! provenance) rides the normal project snapshot recorded for
//! `RelinkMessage::Imported(Ok)` (see `undo::classify`), so a relink is
//! reversible through the same replay path as every other project edit.
//! The `missing` flag itself is re-derived from disk on replay; because a
//! successful relink leaves the WAV in the project folder, an undo reverts
//! the asset's recorded source path/metadata but does not re-hide the
//! now-present file — the project stays self-contained.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use iced::Task;
use resonance_audio::types::AudioCommand;
use resonance_audio::PoolImportOutcome;

use crate::message::{Message, RelinkError, RelinkMessage};
use crate::state::pool::PoolAsset;
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: RelinkMessage) -> Task<Message> {
    match m {
        RelinkMessage::Locate(asset_id) => return locate_dialog(asset_id),
        RelinkMessage::Located(asset_id, picked) => {
            if let Some(path) = picked {
                return start_relink(r, asset_id, path);
            }
        }
        RelinkMessage::SearchFolder => return folder_dialog(),
        RelinkMessage::FolderChosen(picked) => {
            if let Some(folder) = picked {
                return start_batch_relink(r, &folder);
            }
        }
        RelinkMessage::Imported(result) => apply_import(r, result),
        RelinkMessage::ShowModal => {
            // Snapshot the currently-missing assets so the modal can show
            // just-relinked rows as resolved instead of making them vanish.
            let targets: Vec<resonance_audio::types::AssetId> =
                r.pool.missing_assets().map(|a| a.id).collect();
            r.relink.open_modal(targets);
        }
        RelinkMessage::DismissModal => r.relink.close_modal(),
    }
    Task::none()
}

/// Audio container extensions a relink source may use — the same set the
/// import-to-pool and reference loaders accept.
pub const RELINK_AUDIO_EXTENSIONS: &[&str] = &["wav", "flac", "mp3", "ogg"];

/// Open the OS file picker to locate a replacement for a single missing
/// asset. The chosen path (or `None` on cancel) returns as
/// [`RelinkMessage::Located`].
fn locate_dialog(asset_id: resonance_audio::types::AssetId) -> Task<Message> {
    Task::perform(
        async move {
            rfd::AsyncFileDialog::new()
                .set_title("Locate Missing Audio File")
                .add_filter("Audio", RELINK_AUDIO_EXTENSIONS)
                .pick_file()
                .await
                .map(|f| f.path().to_path_buf())
        },
        move |picked| Message::Relink(RelinkMessage::Located(asset_id, picked)),
    )
}

/// Open the OS folder picker for the one-shot batch relink. The chosen
/// folder (or `None` on cancel) returns as [`RelinkMessage::FolderChosen`].
fn folder_dialog() -> Task<Message> {
    Task::perform(
        async move {
            rfd::AsyncFileDialog::new()
                .set_title("Search a Folder for Missing Files")
                .pick_folder()
                .await
                .map(|f| f.path().to_path_buf())
        },
        |picked| Message::Relink(RelinkMessage::FolderChosen(picked)),
    )
}

/// Kick off a relink import for one missing asset against `src_path`.
/// No-op (returns an empty task) when the asset is unknown, is not
/// actually missing, already has a relink in flight, or there is no
/// project directory to copy into.
pub(crate) fn start_relink(
    r: &mut Resonance,
    asset_id: resonance_audio::types::AssetId,
    src_path: PathBuf,
) -> Task<Message> {
    // Only relink an asset that is genuinely missing; ignore a stale
    // request for one that has since been resolved.
    match r.pool.asset(asset_id) {
        Some(a) if a.missing => {}
        _ => return Task::none(),
    }
    if r.relink.is_in_flight(asset_id) {
        return Task::none();
    }
    let Some(project_dir) = r.io.project_path.clone() else {
        r.relink.last_error =
            Some("Cannot relink: the project has not been saved to a folder yet.".into());
        return Task::none();
    };

    r.relink.in_flight.insert(asset_id);
    r.relink.last_error = None;
    spawn_import(asset_id, src_path, project_dir, r.sample_rate)
}

/// Resolve every missing asset whose original filename is found somewhere
/// under `folder` (recursive, case-insensitive), and start a relink
/// import for each. Files not found are left missing. Returns a batch of
/// the spawned import tasks.
pub(crate) fn start_batch_relink(r: &mut Resonance, folder: &Path) -> Task<Message> {
    let Some(project_dir) = r.io.project_path.clone() else {
        r.relink.last_error =
            Some("Cannot relink: the project has not been saved to a folder yet.".into());
        return Task::none();
    };

    // Collect the filenames we're looking for, keyed by asset. An asset
    // whose original path has no filename component (shouldn't happen) is
    // simply skipped.
    let wanted: Vec<(resonance_audio::types::AssetId, String)> = r
        .pool
        .missing_assets()
        .filter(|a| !r.relink.is_in_flight(a.id))
        .filter_map(|a| asset_file_name(a).map(|name| (a.id, name)))
        .collect();
    if wanted.is_empty() {
        return Task::none();
    }

    let names: Vec<String> = wanted.iter().map(|(_, n)| n.clone()).collect();
    let found = scan_folder_for_names(folder, &names);

    let sample_rate = r.sample_rate;
    let mut tasks = Vec::new();
    for (asset_id, name) in wanted {
        if let Some(src) = found.get(&name.to_ascii_lowercase()) {
            r.relink.in_flight.insert(asset_id);
            tasks.push(spawn_import(
                asset_id,
                src.clone(),
                project_dir.clone(),
                sample_rate,
            ));
        }
    }
    if tasks.is_empty() {
        return Task::none();
    }
    r.relink.last_error = None;
    Task::batch(tasks)
}

/// The filename (final path component) of an asset's *original* source,
/// used to match candidates during a batch folder search.
fn asset_file_name(asset: &PoolAsset) -> Option<String> {
    Path::new(&asset.original_path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

/// Build the background copy/transcode task for one asset. Runs
/// [`resonance_audio::import_one_to_pool`] on a blocking worker (it
/// decodes, resamples, and writes the WAV) and reports the outcome as
/// [`RelinkMessage::Imported`].
fn spawn_import(
    asset_id: resonance_audio::types::AssetId,
    src_path: PathBuf,
    project_dir: PathBuf,
    engine_rate: u32,
) -> Task<Message> {
    let src_display = src_path.to_string_lossy().into_owned();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                resonance_audio::import_one_to_pool(
                    asset_id,
                    &src_path.to_string_lossy(),
                    &project_dir,
                    engine_rate,
                )
            })
            .await
            .unwrap_or_else(|join_err| Err(format!("relink import task join: {join_err}")))
        },
        move |result| {
            let mapped = result.map_err(|reason| RelinkError {
                asset_id,
                path: src_display.clone(),
                reason,
            });
            Message::Relink(RelinkMessage::Imported(mapped))
        },
    )
}

/// Apply the outcome of a relink import: on success, refresh the pool
/// asset and reload its clips; on failure, surface the error. Either way
/// the asset leaves the in-flight set.
fn apply_import(r: &mut Resonance, result: Result<PoolImportOutcome, RelinkError>) {
    match result {
        Ok(outcome) => {
            r.relink.in_flight.remove(&outcome.asset_id);
            apply_relinked_asset(r, outcome);
        }
        Err(err) => {
            r.relink.in_flight.remove(&err.asset_id);
            r.relink.last_error = Some(format!("Relink failed for {}: {}", err.path, err.reason));
        }
    }
}

/// Update a pool asset from a successful relink and reload its clips so
/// playback resumes. The WAV has already been copied into the project's
/// `audio/` folder by the import; here we clear the missing flag, refresh
/// the source provenance/duration/waveform, and re-issue
/// [`AudioCommand::LoadClipFromWav`] for every clip that references the
/// asset so the engine memory-maps the now-present file.
pub(crate) fn apply_relinked_asset(r: &mut Resonance, outcome: PoolImportOutcome) {
    let Some(asset) = r.pool.asset_mut(outcome.asset_id) else {
        // The asset was removed (e.g. deleted from the pool) while the
        // import ran — nothing to relink onto. The WAV on disk is
        // harmless; it'll be ignored.
        return;
    };
    asset.project_relative_path = outcome.project_relative_path.clone();
    asset.original_path = outcome.original_path;
    asset.format = outcome.format;
    asset.channels = outcome.channels;
    asset.source_sample_rate = outcome.source_sample_rate;
    asset.duration_frames = outcome.duration_frames;
    asset.thumbnail_peaks = outcome.peaks;
    asset.missing = false;

    // Reload every clip placed from this asset so the engine maps the
    // freshly-restored WAV and audio plays again. Clips carry their own
    // trim/placement; we point them at the asset's project-relative WAV.
    if let Some(project_dir) = r.io.project_path.clone() {
        let wav_path = project_dir.join(&outcome.project_relative_path);
        let reloads: Vec<AudioCommand> = r
            .clips
            .iter()
            .filter(|c| c.asset_ref.map(|a| a.asset_id) == Some(outcome.asset_id))
            .map(|c| AudioCommand::LoadClipFromWav {
                clip_id: c.id,
                track_id: c.track_id,
                start_sample: c.start_sample,
                path: wav_path.clone(),
                name: c.name.clone(),
                trim_start_frames: c.trim_start_frames,
                trim_end_frames: c.trim_end_frames,
            })
            .collect();
        for cmd in reloads {
            let _ = r.engine.send(cmd);
        }
    }

    r.recompute_pool_usage();
}

/// Recursively walk `folder` looking for files whose name (case-
/// insensitive) is one of `names`. Returns a map from the lowercased
/// filename to the first matching path found. The walk is bounded by a
/// depth guard and never follows into unreadable directories, so a huge or
/// permission-restricted tree can't hang or panic the search.
pub fn scan_folder_for_names(folder: &Path, names: &[String]) -> HashMap<String, PathBuf> {
    /// Cap on directory-tree depth so a pathological / cyclic layout
    /// (symlink loops) can't spin forever.
    const MAX_DEPTH: usize = 24;

    let wanted: std::collections::HashSet<String> =
        names.iter().map(|n| n.to_ascii_lowercase()).collect();
    let mut found: HashMap<String, PathBuf> = HashMap::new();

    // Iterative DFS with an explicit stack of (dir, depth).
    let mut stack: Vec<(PathBuf, usize)> = vec![(folder.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        // Every wanted name already resolved — stop early.
        if found.len() == wanted.len() {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if depth < MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if let Some(name) = path.file_name() {
                let key = name.to_string_lossy().to_ascii_lowercase();
                if wanted.contains(&key) {
                    found.entry(key).or_insert(path);
                }
            }
        }
    }
    found
}
