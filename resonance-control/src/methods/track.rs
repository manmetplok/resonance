//! `track.*` — track lifecycle and per-track plugins.
//!
//! Volume/pan/mute/solo live under `mixer.*`. Simple mutations return
//! [`crate::common::MutationAck`].

use crate::common::{TrackKind, TrackOutput};
use crate::ids::TrackId;
use serde::{Deserialize, Serialize};

/// `track.add` — add a track ([`AddParams`] -> [`AddResult`]).
pub const ADD: &str = "track.add";
/// `track.rename` — rename a track ([`RenameParams`] -> `MutationAck`).
pub const RENAME: &str = "track.rename";
/// `track.delete` — delete a track; destructive, requires
/// `"confirm": true` ([`DeleteParams`] -> `MutationAck`).
pub const DELETE: &str = "track.delete";
/// `track.add_instrument` — set a built-in instrument by stable plugin
/// id ([`AddPluginParams`] -> `MutationAck`).
pub const ADD_INSTRUMENT: &str = "track.add_instrument";
/// `track.add_effect` — append a built-in effect to the insert chain
/// ([`AddPluginParams`] -> `MutationAck`).
pub const ADD_EFFECT: &str = "track.add_effect";
/// `track.remove_effect` — take an effect off the insert chain
/// ([`RemoveEffectParams`] -> `MutationAck`).
pub const REMOVE_EFFECT: &str = "track.remove_effect";
/// `track.set_output` — route a track to master or into a bus
/// ([`SetOutputParams`] -> `MutationAck`).
pub const SET_OUTPUT: &str = "track.set_output";
/// `track.plugins` — the built-in plugin catalog, read-only
/// (no params -> [`PluginCatalog`]).
pub const PLUGINS: &str = "track.plugins";

/// `track.plugin_params` — a track's plugins and their parameters,
/// read-only ([`PluginParamsParams`] -> [`PluginParamsView`]).
pub const PLUGIN_PARAMS: &str = "track.plugin_params";
/// `track.set_plugin_param` — set one plugin parameter
/// ([`SetPluginParamParams`] -> `MutationAck`).
pub const SET_PLUGIN_PARAM: &str = "track.set_plugin_param";

/// All `track.*` method names.
pub const METHODS: &[&str] = &[
    ADD,
    RENAME,
    DELETE,
    ADD_INSTRUMENT,
    ADD_EFFECT,
    REMOVE_EFFECT,
    SET_OUTPUT,
    PLUGINS,
    PLUGIN_PARAMS,
    SET_PLUGIN_PARAM,
];

/// Params for `track.add`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddParams {
    pub kind: TrackKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Result of `track.add`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddResult {
    pub track_id: TrackId,
    pub revision: u64,
}

/// Params for `track.rename`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RenameParams {
    pub track_id: TrackId,
    pub name: String,
}

/// Params for `track.delete`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DeleteParams {
    pub track_id: TrackId,
    /// Required (`true`); the error otherwise summarizes what would be lost.
    #[serde(default)]
    pub confirm: bool,
}

/// Params for `track.add_instrument` / `track.add_effect`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddPluginParams {
    pub track_id: TrackId,
    /// Stable plugin id from the catalog, e.g. `"resonance-wavetable"`.
    pub plugin_id: String,
}

/// Result of `track.plugins`: the built-in plugin catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginCatalog {
    pub plugins: Vec<PluginCatalogEntry>,
}

/// One catalog entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginCatalogEntry {
    /// Stable id used in [`AddPluginParams::plugin_id`].
    pub id: String,
    pub name: String,
    pub kind: PluginKind,
}

/// Plugin roles, lowercase on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    Instrument,
    Effect,
}

// ---------------------------------------------------------------------------
// Plugin parameters (ba doc #272 V-3)
// ---------------------------------------------------------------------------

/// Params for `track.plugin_params`.
///
/// A plugin is addressed by the CLAP id `song.tracks` already reports —
/// its `instrument` field or an entry of its `effects` array — so no
/// extra identifier has to be discovered first. A track carrying the
/// same plugin twice disambiguates with `occurrence`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginParamsParams {
    pub track_id: TrackId,
    /// CLAP id of the plugin on this track, e.g.
    /// `"com.resonance.wavetable"`. Omitted returns every plugin on the
    /// track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance to address when the track carries `plugin_id`
    /// more than once; 0-based, defaults to the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

/// Result of `track.plugin_params`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginParamsView {
    pub track_id: TrackId,
    pub plugins: Vec<PluginParamsEntry>,
    pub revision: u64,
}

/// One plugin on the track, with its parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginParamsEntry {
    /// CLAP id — what [`PluginParamsParams::plugin_id`] and
    /// [`SetPluginParamParams::plugin_id`] take.
    pub plugin_id: String,
    pub name: String,
    /// 0-based position in the track's insert chain, the instrument
    /// included, so slot order IS processing order. `slot` and
    /// `(plugin_id, occurrence)` are two ways to address the same
    /// plugin: `slot` says *where* it sits, the pair says *which* copy
    /// of a repeated effect it is. Slots renumber when a plugin is
    /// removed, so re-read this view between removals.
    #[serde(default)]
    pub slot: u32,
    /// 0-based position of this plugin among the track's instances of
    /// the same `plugin_id` — pass as `occurrence` to address it.
    pub occurrence: u32,
    pub kind: PluginKind,
    pub params: Vec<PluginParamView>,
}

/// Params for `track.set_output` — where a track's post-fader audio
/// goes.
///
/// `song.summary` / `song.tracks` report the current destination as the
/// same [`TrackOutput`] shape, so a read and a write use one vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetOutputParams {
    pub track_id: TrackId,
    /// `"master"` to sum directly into the master output, or
    /// `{"bus_id": N}` to route through a group bus first.
    pub output: TrackOutput,
}

/// Params for `track.remove_effect`: address the plugin **either** by
/// `slot` **or** by `plugin_id` (+ `occurrence`). Both forms together,
/// or neither, is `invalid_params` — with `track.add_effect` appending a
/// fresh instance on every call, guessing which copy was meant is how
/// the wrong processor comes off.
///
/// The track's INSTRUMENT cannot be removed this way; replace it with
/// `track.add_instrument` instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RemoveEffectParams {
    pub track_id: TrackId,
    /// 0-based chain position, as reported by
    /// [`PluginParamsEntry::slot`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// CLAP id of the effect to remove.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based; defaults to the first.
    /// Only meaningful together with `plugin_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

/// One plugin parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginParamView {
    /// Stable CLAP parameter id. Either this (as a decimal string) or
    /// `name` addresses the parameter in `track.set_plugin_param`.
    pub id: u32,
    pub name: String,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub default: f64,
}

/// Params for `track.set_plugin_param`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetPluginParamParams {
    pub track_id: TrackId,
    /// CLAP id of the plugin to address; omitted targets the track's
    /// instrument.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// The parameter, by name (case-insensitive) or by its numeric id
    /// as a string. Names come from `track.plugin_params`.
    pub param: String,
    /// New value; must lie within the parameter's `min..=max`, which an
    /// out-of-range request reports back.
    pub value: f64,
}
