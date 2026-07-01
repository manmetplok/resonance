//! Media-pool data model: imported audio assets, their per-asset usage
//! counts (derived from the clips that reference them), and the
//! favourite / recent-folder lists shown alongside the pool in the media
//! browser.
//!
//! Data model first landed in todo #595 (app state + undo). Persistence
//! (this is todo #596) makes the durable parts survive save/reload:
//!
//! * **Pool assets + clip asset refs** are part of the on-disk
//!   `ProjectFile` and ride the normal replay / replay-diff path (the
//!   same channel tracks, clips, and markers use). Serialized in
//!   `update::project_io::serialize`, rebuilt in
//!   `update::project_io::replay` (`restore_pool`) and
//!   `update::project_io::replay_diff` (`apply_pool`). An asset whose
//!   backing WAV is absent on load is flagged [`PoolAsset::missing`] —
//!   kept, not dropped — so its clips can be relinked later.
//! * **Favourites + recent folders** are *project-independent* user
//!   state and persist in `settings.json`
//!   ([`crate::settings::MediaBrowserSettings`]), not in any project.
//!   They are loaded into the pool at startup and written back whenever
//!   they change. The pool keeps the live working copy here so the
//!   browser UI (todos #599 / #602) can read them directly.
//!
//! Undo: pool assets / asset refs ride the `ProjectFile` snapshot the
//! undo history already takes, so add / remove / relink are reversible
//! through the same replay path as every other project edit. Favourites
//! and recent folders are deliberately *not* undoable — like the
//! browser's transient audition state, they're user-/session-level, not
//! project edits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use resonance_audio::types::{AssetId, ClipId};
use resonance_common::AudioFormat;

/// Most-recent-first cap on the recent-folders list. Older entries fall
/// off the end as new folders are visited.
pub const RECENT_FOLDERS_CAP: usize = 12;

/// One imported audio file in the project's media pool. The engine
/// transcodes every import to a project-rate stereo f32 WAV living at
/// `project_relative_path`; the remaining metadata fields describe the
/// *original* source, for display.
#[derive(Debug, Clone, PartialEq)]
pub struct PoolAsset {
    pub id: AssetId,
    /// Path of the engine-format WAV inside the project directory, e.g.
    /// `"audio/asset_7.wav"`.
    pub project_relative_path: String,
    /// Absolute path of the source file the user imported.
    pub original_path: String,
    /// Container/codec family of the original source.
    pub format: AudioFormat,
    /// Channel count of the original source file.
    pub channels: u16,
    /// Sample rate of the original source file, in Hz.
    pub source_sample_rate: u32,
    /// Per-channel frame count of the imported (project-rate) WAV — what
    /// a clip placed from this asset would span.
    pub duration_frames: u64,
    /// Downsampled waveform peaks: (min, max) per chunk of frames. Sized
    /// for a browser-row thumbnail.
    pub thumbnail_peaks: Vec<(f32, f32)>,
    /// True when the backing WAV is no longer present on disk (e.g. the
    /// project was moved without its `audio/` folder). A missing asset is
    /// kept in the pool — flagged, not dropped — so its clips can be
    /// relinked once the file is located again.
    pub missing: bool,
}

/// Links an `AudioClip` to the pool asset it was placed from. Stored on
/// [`crate::state::ClipState`]; an asset's usage count is the number of
/// clips whose `AssetRef` points at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetRef {
    pub asset_id: AssetId,
}

impl AssetRef {
    pub fn new(asset_id: AssetId) -> Self {
        Self { asset_id }
    }
}

/// The project's media pool plus the browser's favourite and recent
/// folder lists. Held in `Resonance` state. The `assets` ride the
/// project file; `favourites` / `recent_folders` ride user settings.
#[derive(Debug, Clone, Default)]
pub struct MediaPool {
    /// Imported assets, in import order.
    pub assets: Vec<PoolAsset>,
    /// Per-asset usage count, recomputed from clip references by
    /// [`MediaPool::recompute_usage`]. An asset absent from this map has
    /// a usage count of zero.
    usage: HashMap<AssetId, u32>,
    /// User-pinned folder paths shown at the top of the media browser.
    /// Persisted in user settings, not in the project.
    pub favourites: Vec<PathBuf>,
    /// Recently-visited folder paths, most-recent first, capped at
    /// [`RECENT_FOLDERS_CAP`]. Persisted in user settings, not in the
    /// project.
    pub recent_folders: Vec<PathBuf>,
}

impl MediaPool {
    /// Build a pool seeded with the user's persisted favourite / recent
    /// folder lists (from `settings.json`) and an empty asset set. Used
    /// at app startup; project assets are filled in later by the load
    /// path. Keeps `usage` private — callers can't construct the field
    /// directly across the module boundary.
    pub fn with_user_folders(favourites: Vec<PathBuf>, recent_folders: Vec<PathBuf>) -> Self {
        Self {
            favourites,
            recent_folders,
            ..Self::default()
        }
    }

    /// Borrow the asset with the given id, if present.
    pub fn asset(&self, id: AssetId) -> Option<&PoolAsset> {
        self.assets.iter().find(|a| a.id == id)
    }

    /// Mutably borrow the asset with the given id, if present.
    pub fn asset_mut(&mut self, id: AssetId) -> Option<&mut PoolAsset> {
        self.assets.iter_mut().find(|a| a.id == id)
    }

    /// True when an asset with this id is in the pool.
    pub fn contains(&self, id: AssetId) -> bool {
        self.assets.iter().any(|a| a.id == id)
    }

    /// Every asset whose backing WAV is currently flagged missing, in
    /// import order. Drives the relink flow (doc #175, todo #600): the
    /// batch "Search a folder…" resolves this whole set by filename, and
    /// the browser's Pool tab renders a `relink` chip per entry.
    pub fn missing_assets(&self) -> impl Iterator<Item = &PoolAsset> {
        self.assets.iter().filter(|a| a.missing)
    }

    /// True when any pool asset is flagged missing.
    pub fn has_missing(&self) -> bool {
        self.assets.iter().any(|a| a.missing)
    }

    /// Add an asset to the pool. If an asset with the same id already
    /// exists it is replaced in place (re-import / metadata refresh),
    /// preserving its position; otherwise the asset is appended.
    pub fn add(&mut self, asset: PoolAsset) {
        if let Some(slot) = self.assets.iter_mut().find(|a| a.id == asset.id) {
            *slot = asset;
        } else {
            self.assets.push(asset);
        }
    }

    /// Remove an asset from the pool, returning it if it was present. Its
    /// usage entry is dropped too; any clips still referencing it become
    /// dangling (their `AssetRef` points at a now-absent asset) and are
    /// no longer counted.
    pub fn remove(&mut self, id: AssetId) -> Option<PoolAsset> {
        let pos = self.assets.iter().position(|a| a.id == id)?;
        self.usage.remove(&id);
        Some(self.assets.remove(pos))
    }

    /// Drop every asset and usage entry. Called when a project is loaded
    /// so the previous project's pool doesn't leak into the new one.
    /// Leaves favourites / recent folders untouched — those are
    /// user-level, not project-level, state.
    pub fn clear_assets(&mut self) {
        self.assets.clear();
        self.usage.clear();
    }

    /// The highest asset id currently in the pool, if any. Lets a loader
    /// bump the engine's id counter past restored ids so freshly
    /// imported assets never collide with persisted ones.
    pub fn max_asset_id(&self) -> Option<AssetId> {
        self.assets.iter().map(|a| a.id).max()
    }

    /// Per-asset usage count. Zero for assets that aren't referenced by
    /// any clip (or aren't in the pool at all).
    pub fn usage_count(&self, id: AssetId) -> u32 {
        self.usage.get(&id).copied().unwrap_or(0)
    }

    /// Recompute every asset's usage count from the supplied clip
    /// references. Counts are derived purely from the clips, so this is
    /// the single source of truth — call it after any clip add/remove or
    /// relink. References to assets not in the pool are ignored.
    pub fn recompute_usage<I>(&mut self, refs: I)
    where
        I: IntoIterator<Item = AssetId>,
    {
        self.usage.clear();
        for asset_id in refs {
            if self.contains(asset_id) {
                *self.usage.entry(asset_id).or_insert(0) += 1;
            }
        }
    }

    // -- Favourites ----------------------------------------------------

    /// True when `path` is pinned as a favourite folder.
    pub fn is_favourite(&self, path: &Path) -> bool {
        self.favourites.iter().any(|p| p == path)
    }

    /// Pin a folder as a favourite. No-op if already pinned.
    pub fn add_favourite(&mut self, path: PathBuf) {
        if !self.is_favourite(&path) {
            self.favourites.push(path);
        }
    }

    /// Unpin a favourite folder. No-op if it wasn't pinned.
    pub fn remove_favourite(&mut self, path: &Path) {
        self.favourites.retain(|p| p != path);
    }

    /// Toggle a folder's favourite state, returning its new state.
    pub fn toggle_favourite(&mut self, path: PathBuf) -> bool {
        if self.is_favourite(&path) {
            self.remove_favourite(&path);
            false
        } else {
            self.favourites.push(path);
            true
        }
    }

    // -- Recent folders ------------------------------------------------

    /// Record a folder as most-recently visited. Moves an existing entry
    /// to the front rather than duplicating it, and trims the list to
    /// [`RECENT_FOLDERS_CAP`].
    pub fn push_recent_folder(&mut self, path: PathBuf) {
        self.recent_folders.retain(|p| p != &path);
        self.recent_folders.insert(0, path);
        self.recent_folders.truncate(RECENT_FOLDERS_CAP);
    }
}

impl crate::Resonance {
    /// Recompute the pool's per-asset usage counts from the current
    /// clip set. Called after any pool/clip mutation that could change
    /// which assets are referenced.
    pub(crate) fn recompute_pool_usage(&mut self) {
        let refs = self
            .clips
            .iter()
            .filter_map(|c| c.asset_ref.map(|r| r.asset_id))
            .collect::<Vec<_>>();
        self.pool.recompute_usage(refs);
    }

    /// Add an imported asset to the pool and refresh usage counts.
    pub(crate) fn add_pool_asset(&mut self, asset: PoolAsset) {
        self.pool.add(asset);
        self.recompute_pool_usage();
    }

    /// Remove an asset from the pool, returning it if present, and
    /// refresh usage counts.
    pub(crate) fn remove_pool_asset(&mut self, id: AssetId) -> Option<PoolAsset> {
        let removed = self.pool.remove(id);
        self.recompute_pool_usage();
        removed
    }

    /// Point a clip at a pool asset (or clear its link when `asset_id` is
    /// `None`) and refresh usage counts. No-op if the clip id is unknown.
    pub(crate) fn relink_clip(&mut self, clip_id: ClipId, asset_id: Option<AssetId>) {
        if let Some(clip) = self.clips.iter_mut().find(|c| c.id == clip_id) {
            clip.asset_ref = asset_id.map(AssetRef::new);
        }
        self.recompute_pool_usage();
    }

    /// Copy the pool's favourite / recent-folder lists into the app's
    /// in-memory settings document. Pure (no disk I/O) so it's unit
    /// testable; [`Self::persist_media_browser_settings`] calls this then
    /// writes the document out.
    pub(crate) fn sync_media_browser_settings(&mut self) {
        self.settings.media.favourites = self.pool.favourites.clone();
        self.settings.media.recent_folders = self.pool.recent_folders.clone();
    }

    /// Mirror the pool's favourite / recent-folder lists into user
    /// settings and persist them to disk. Favourites and recent folders
    /// are project-independent (doc #175), so they live in `settings.json`
    /// rather than any project file. Call this after any favourite /
    /// recent mutation so the change survives the session.
    pub(crate) fn persist_media_browser_settings(&mut self) {
        self.sync_media_browser_settings();
        crate::settings::persist(&self.settings);
    }
}
