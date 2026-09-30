//! `amp_models.*` — the per-user NAM model library that Resonance Amp
//! loads from (nam-model-library.md §9.3).
//!
//! A query about the USER's installed models, not about the project: it
//! needs no open project and no running amp instance, and it never touches
//! the undo history or the project `revision`. The app reads the same files
//! the plugin does (`resonance_common::nam_library`).
//!
//! An agent picks a model by setting the amp's `Model Select` parameter to
//! an entry's `slot` (or to its `name`, which the plugin resolves) with
//! `track.set_plugin_param`.

use serde::{Deserialize, Serialize};

/// `amp_models.list` — the installed models ([`ListParams`] ->
/// [`AmpModelList`]). Read-only.
pub const LIST: &str = "amp_models.list";

/// `amp_models.set_marks` — favourite or tag one model ([`SetMarksParams`]
/// -> [`AmpModelEntry`]). Per-user state, not the project: no undo entry,
/// no `revision` bump.
pub const SET_MARKS: &str = "amp_models.set_marks";

/// All `amp_models.*` method names — what `control.hello` advertises.
pub const METHODS: &[&str] = &[LIST, SET_MARKS];

/// Params for `amp_models.list`. Every filter is optional and they AND.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ListParams {
    /// Search text, the same syntax as the amp's Library panel: tokens are
    /// ANDed and matched (case- and accent-insensitively) against name,
    /// author, gear, file name and tags; `is:fav`, `is:recent`, `tag:<t>`,
    /// `by:<author>`, `is:tone3000` / `is:imported`, `gear_type:<g>` and
    /// `tone_type:<t>` scope a token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Only favourites.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub favorites_only: bool,
    /// Only this gear type, as the file names it (`amp`, `amp_cab`,
    /// `pedal`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gear_type: Option<String>,
    /// Only this capture type (`clean`, `crunch`, `hi_gain`, `fuzz`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone_type: Option<String>,
    /// At most this many models (the first ones in the list's order);
    /// `matched` still counts them all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// The health of one installed model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum AmpModelStatus {
    /// Loadable.
    Ok,
    /// The file does not parse; `error` says why.
    Unreadable,
    /// The same bytes as another entry, which holds the slot.
    Duplicate,
}

/// Where a model came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AmpModelSource {
    /// Downloaded from Tone3000 (the amp editor can re-download it).
    Tone3000 { tone_id: i64, model_id: i64 },
    /// Copied into the library by Import.
    Imported,
    /// Dropped into the library folder by hand.
    External,
}

/// One installed model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AmpModelEntry {
    /// The value of Resonance Amp's `Model Select` (`file_select`) that
    /// loads this model. Stable: adding or deleting other models never
    /// moves it. Absent for a byte-identical duplicate (its original holds
    /// the slot) and past the 1000th model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// Content id: sha256 of the file. What `amp_models.set_marks` takes.
    pub id: String,
    /// Display name; also accepted by `track.set_plugin_param` on Model
    /// Select when no other installed model has the same name (prefer the
    /// slot or id, which are unambiguous).
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Make and model of the captured gear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gear: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gear_type: Option<String>,
    /// Capture type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone_type: Option<String>,
    /// `WaveNet A1`, `WaveNet A2`, `A2 slimmable`, `LSTM`, …
    pub architecture: String,
    /// The rate the model was captured at. A model runs sample-for-sample,
    /// so at another session rate its tone shifts.
    pub sample_rate: f64,
    pub size_bytes: u64,
    pub source: AmpModelSource,
    pub favorite: bool,
    /// Personal tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// RFC 3339; when the user last picked it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
    pub status: AmpModelStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Params for `amp_models.set_marks`. At least one of `favorite` / `tags`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetMarksParams {
    /// The model's content id from `amp_models.list` (a unique prefix of
    /// at least 8 characters also works).
    pub id: String,
    /// Star or un-star it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
    /// REPLACE its personal tags (normalised to lowercase `a-z0-9-`); `[]`
    /// clears them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// Result of `amp_models.list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AmpModelList {
    /// Matching models: favourites first, then slot order (at most
    /// `limit`).
    pub models: Vec<AmpModelEntry>,
    /// How many models matched the filters, before `limit`.
    pub matched: usize,
    /// Bumped whenever the library's index changes (any process).
    pub library_generation: u64,
    /// How many models are installed in all, before the filters.
    pub total: usize,
}
