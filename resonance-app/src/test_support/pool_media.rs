//! Media pool and browser test hooks: pool assets, relinking,
//! favourites/recents, audition transport and import progress.

use crate::state;
use crate::Resonance;

impl Resonance {
    /// Test-only: number of import placements still awaiting their
    /// `AssetImported` event (ba todo #598).
    #[doc(hidden)]
    pub fn test_pending_import_count(&self) -> usize {
        self.pool_import.len()
    }

    /// Test-only: borrow the media pool so persistence / mirror tests can
    /// assert the restored asset list, missing flags, usage counts, and
    /// favourite / recent folder lists.
    #[doc(hidden)]
    pub fn test_pool(&self) -> &crate::state::MediaPool {
        &self.pool
    }

    /// Test-only: add an imported asset to the pool (and refresh usage),
    /// standing in for the import-to-pool orchestration (ba todo #598)
    /// so a persistence test can seed a pool before serializing.
    #[doc(hidden)]
    pub fn test_add_pool_asset(&mut self, asset: crate::state::PoolAsset) {
        self.add_pool_asset(asset);
    }

    /// Test-only: remove an asset from the pool, returning it if present.
    #[doc(hidden)]
    pub fn test_remove_pool_asset(
        &mut self,
        id: resonance_audio::types::AssetId,
    ) -> Option<crate::state::PoolAsset> {
        self.remove_pool_asset(id)
    }

    /// Test-only: point a clip at a pool asset (or clear the link with
    /// `None`) and refresh usage, standing in for the placement /
    /// relink handlers (ba todos #598 / #600).
    #[doc(hidden)]
    pub fn test_relink_clip(
        &mut self,
        clip_id: resonance_audio::types::ClipId,
        asset_id: Option<resonance_audio::types::AssetId>,
    ) {
        self.relink_clip(clip_id, asset_id);
    }

    /// Test-only: replay just the media-pool block of a saved
    /// [`crate::project::ProjectFile`] into this app, resolving relative
    /// asset paths against `project_dir`. Exercises the same restore path
    /// a full project load runs (missing-file flagging, usage recompute)
    /// without constructing a whole `LoadedProject`.
    #[doc(hidden)]
    pub fn test_restore_pool(
        &mut self,
        file: &crate::project::ProjectFile,
        project_dir: &std::path::Path,
    ) {
        crate::update::project_io::restore_pool(self, file, project_dir);
    }

    /// Test-only: mirror the pool's favourites / recent folders into app
    /// settings *without* writing to disk (doc #175). Pairs with
    /// [`Self::test_settings`] to assert the synced lists in a hermetic
    /// test — unlike [`Self::test_persist_media_browser_settings`], this
    /// never touches the real `config_dir()`.
    #[doc(hidden)]
    pub fn test_sync_media_browser_settings(&mut self) {
        self.sync_media_browser_settings();
    }

    /// Test-only: mirror the pool's favourites / recent folders into app
    /// settings and persist them to disk (doc #175), standing in for the
    /// browser favourite/recent handlers (ba todo #599). Writes to the
    /// real `config_dir()`, so prefer [`Self::test_sync_media_browser_settings`]
    /// in tests that only need to assert the in-memory document.
    #[doc(hidden)]
    pub fn test_persist_media_browser_settings(&mut self) {
        self.persist_media_browser_settings();
    }

    /// Test-only: pin a favourite folder on the pool, standing in for the
    /// browser's favourite toggle (ba todo #599).
    #[doc(hidden)]
    pub fn test_pool_add_favourite(&mut self, path: std::path::PathBuf) {
        self.pool.add_favourite(path);
    }

    /// Test-only: record a most-recently-visited folder on the pool,
    /// standing in for the browser's navigation handler (ba todo #599).
    #[doc(hidden)]
    pub fn test_pool_push_recent(&mut self, path: std::path::PathBuf) {
        self.pool.push_recent_folder(path);
    }

    /// Test-only: borrow the transient media-browser state so navigation /
    /// filter / audition handler tests (ba todo #599) can assert the
    /// current folder, cached scan, filter, tab, and audition transport.
    #[doc(hidden)]
    pub fn test_browser(&self) -> &crate::state::BrowserState {
        &self.browser
    }

    /// Test-only: the lazy key of the Files-tab folder listing (review
    /// VIEW-27).
    #[doc(hidden)]
    pub fn test_files_listing_fingerprint(&self) -> u64 {
        crate::view::browser::listing_fingerprint(self)
    }

    /// Test-only: the geometry-cache key of a browser waveform thumbnail
    /// (review VIEW-27).
    #[doc(hidden)]
    pub fn test_wave_thumbnail_key(peaks: &[(f32, f32)], muted: bool) -> u64 {
        crate::view::browser::WaveThumbnail {
            peaks: std::borrow::Cow::Borrowed(peaks),
            muted,
        }
        .cache_key()
    }

    /// Test-only: borrow one pool asset by id, so relink tests (ba todo
    /// #600) can assert an asset's missing flag and refreshed metadata.
    #[doc(hidden)]
    pub fn test_pool_asset(
        &self,
        id: resonance_audio::types::AssetId,
    ) -> Option<&crate::state::pool::PoolAsset> {
        self.pool.asset(id)
    }

    /// Test-only: borrow the transient relink state so relink handler
    /// tests (ba todo #600) can assert in-flight bookkeeping and the last
    /// relink error without poking at the `pub(crate)` field.
    #[doc(hidden)]
    pub fn test_relink(&self) -> &crate::state::RelinkState {
        &self.relink
    }

    /// Test-only: borrow the per-file import-progress tracker so tests can
    /// assert that `ImportProgress` / `ImportFailed` engine events update it
    /// correctly, without touching the private field directly.
    #[doc(hidden)]
    pub fn test_import_progress(&self) -> &state::ImportProgressTracker {
        &self.import_progress
    }

    /// Test-only: whether the audio-import transcode-progress modal is
    /// currently open (ba todo #606). Set to `true` when an import batch
    /// is initiated and cleared by `DismissImportProgress`.
    #[doc(hidden)]
    pub fn test_import_progress_modal_open(&self) -> bool {
        self.import_progress_modal_open
    }

    /// Test-only: directly open or close the import-progress modal without
    /// driving a full import, so snapshot and behavioural tests can set up
    /// a deterministic modal state.
    #[doc(hidden)]
    pub fn test_set_import_progress_modal_open(&mut self, open: bool) {
        self.import_progress_modal_open = open;
    }

    /// Test-only: borrow the audition transport state (playing row, scrub
    /// playhead position) so tests for `AuditionPosition` / `AuditionStopped`
    /// mirroring (ba todo #597) can assert without reading the private field.
    #[doc(hidden)]
    pub fn test_audition(&self) -> &state::AuditionState {
        &self.browser.audition
    }

    /// Test-only: directly set the audition `playing` row, standing in for
    /// the browser `Play` message handler so `AuditionStopped` mirror tests
    /// have a pre-existing playing state to clear.
    #[doc(hidden)]
    pub fn test_set_audition_playing(&mut self, path: Option<std::path::PathBuf>) {
        self.browser.audition.playing = path;
    }
}
