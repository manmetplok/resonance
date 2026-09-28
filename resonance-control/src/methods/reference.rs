//! `reference.*` — reference tracks: commercial masters the user supplies
//! to compare the mix against (warmth-width-depth.md §7.5).
//!
//! A reference lives in the project's A/B reference list — the same list
//! the GUI's reference panel shows, where the user can audition it
//! against the mix, loudness-matched. It comes from the media pool: import
//! the file with `pool.import`, then load the pooled asset here. The
//! pooled copy is the project-rate file a clip placed from the asset
//! plays, so a reference measures exactly like that clip would.
//!
//! Measure a loaded reference with `meter.measure` and the target
//! `{"reference": reference_id}` — every figure and detail block a mix
//! slice gets.

use crate::ids::{AssetId, ReferenceId};
use serde::{Deserialize, Serialize};

/// `reference.load` — add a pooled asset to the project's reference list
/// ([`LoadParams`] -> [`LoadResult`]). Undoable, like loading one in the
/// GUI.
pub const LOAD: &str = "reference.load";

/// All `reference.*` method names.
pub const METHODS: &[&str] = &[LOAD];

/// Params for `reference.load`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LoadParams {
    /// The reference's media-pool asset id, from `pool.list` (import the
    /// file with `pool.import` first).
    pub pool_asset_id: AssetId,
}

/// Result of `reference.load`.
///
/// The reference is decoded in the background: it is listed at once, and
/// measurable (and audible in the A/B panel) a moment later. A
/// `meter.measure` of it before then answers `busy`; retry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LoadResult {
    /// Pass this as `meter.measure`'s `{"reference": reference_id}`.
    pub reference_id: ReferenceId,
    /// The name it is listed under (the asset's).
    pub name: String,
    pub revision: u64,
}
