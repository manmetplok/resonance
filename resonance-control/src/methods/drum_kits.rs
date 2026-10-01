//! `drum_kits.*` — the per-user drum-kit library that Resonance Drums
//! loads from (drums-plugin-rework.md §8). The amp's `amp_models.*` twin.
//!
//! A query about the USER's installed kits, not about the project: it
//! needs no open project and no running drums instance, and it never
//! touches the undo history or the project `revision`. The app reads the
//! same files the plugin does (`resonance_common::drumkit_library`).
//!
//! An agent picks a kit by setting the drums' `kit_select` parameter to an
//! entry's `name` (or its `slot`) with `track.set_plugin_param`, then
//! polls `kit_load_progress` until it reads 1.0.
//!
//! Not exposed: delete, import, download (the NAM library's D6).

use serde::{Deserialize, Serialize};

use crate::ids::TrackId;

/// `drum_kits.list` — the installed kits ([`ListParams`] ->
/// [`DrumKitList`]). Read-only.
pub const LIST: &str = "drum_kits.list";

/// `drum_kits.set_marks` — favourite or tag one kit ([`SetMarksParams`]
/// -> [`DrumKitEntry`]). Per-user state, not the project: no undo entry,
/// no `revision` bump.
pub const SET_MARKS: &str = "drum_kits.set_marks";

/// All `drum_kits.*` method names — what `control.hello` advertises.
pub const METHODS: &[&str] = &[LIST, SET_MARKS];

/// Params for `drum_kits.list`. Every filter is optional and they AND.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ListParams {
    /// Search text, the same syntax as the drums' Library overlay: tokens
    /// are ANDed and matched (case- and accent-insensitively) against the
    /// kit name, its directory, description, piece names, mic brands,
    /// models and positions, and tags; `is:fav`, `is:recent`, `tag:<t>`,
    /// `is:plok` / `is:imported` / `is:local`, `mics:<1|2–4|5+>` and
    /// `articulations:<yes|no>` scope a token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Only favourites.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub favorites_only: bool,
    /// Only kits from this source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<DrumKitSource>,
    /// At most this many kits (the first ones in the list's order);
    /// `matched` still counts them all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// Where a kit came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum DrumKitSource {
    /// Downloaded from plok.org by the drums' Download tab.
    Plok,
    /// Copied into the library by Import.
    Imported,
    /// Dropped into the library folder by hand.
    Local,
}

/// The health of one installed kit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum DrumKitStatus {
    /// Loadable.
    Ok,
    /// Its `drum_samples.json` does not parse; `error` says why.
    ManifestError,
    /// Loadable, but some sample files are missing (`error` counts them);
    /// those pieces stay silent.
    MissingFiles,
    /// The same manifest as another entry, which holds the slot.
    Duplicate,
}

/// A track whose Resonance Drums instance has this kit selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DrumKitUser {
    pub track_id: TrackId,
    pub track_name: String,
}

/// One installed kit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DrumKitEntry {
    /// The value of Resonance Drums' `kit_select` that loads this kit.
    /// Stable: adding or deleting other kits never moves it. Absent for a
    /// duplicate (its original holds the slot) and past the 1000th kit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// Content id: sha256 of the kit's manifest. What
    /// `drum_kits.set_marks` takes.
    pub id: String,
    /// Display name — what `kit_select` reads as its text, and what
    /// `track.set_plugin_param` accepts as its value.
    pub name: String,
    /// The kit's description, when its download carried one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// How many pieces (drums and cymbals, each articulation its own
    /// piece) the kit has.
    pub pieces: usize,
    /// The pieces' display names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub piece_names: Vec<String>,
    /// How many mic setups (close mics, overheads, room) it was recorded
    /// with.
    pub mic_setups: usize,
    /// The most velocity layers any piece has.
    pub layers: u32,
    /// The most round-robin takes any layer has.
    pub rr: u32,
    /// On-disk size, once measured (the drums plugin measures a kit
    /// without a download record in the background).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    pub source: DrumKitSource,
    pub favorite: bool,
    /// Personal tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// RFC 3339; when the user last picked it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
    pub status: DrumKitStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The tracks of the open project whose Resonance Drums has this kit
    /// selected (by `kit_select` slot); empty with no project open.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loaded_in: Vec<DrumKitUser>,
}

/// Params for `drum_kits.set_marks`. At least one of `favorite` / `tags`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetMarksParams {
    /// The kit's content id from `drum_kits.list` (a unique prefix of at
    /// least 8 characters also works).
    pub id: String,
    /// Star or un-star it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
    /// REPLACE its personal tags (normalised to lowercase `a-z0-9-`); `[]`
    /// clears them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// Result of `drum_kits.list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DrumKitList {
    /// Matching kits: favourites first, then slot order (at most
    /// `limit`).
    pub kits: Vec<DrumKitEntry>,
    /// How many kits matched the filters, before `limit`.
    pub matched: usize,
    /// Bumped whenever the library's index changes (any process).
    pub library_generation: u64,
    /// How many kits are installed in all, before the filters.
    pub total: usize,
}
