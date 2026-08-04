//! `track.*` — track lifecycle and per-track plugins.
//!
//! Volume/pan/mute/solo live under `mixer.*`. Simple mutations return
//! [`crate::common::MutationAck`].

use crate::common::{TrackKind, TrackOutput};
use crate::ids::{SendId, TrackId};
use serde::{Deserialize, Serialize};

/// `track.add` — add a track ([`AddParams`] -> [`AddResult`]).
pub const ADD: &str = "track.add";
/// `track.rename` — rename a track ([`RenameParams`] -> `MutationAck`).
pub const RENAME: &str = "track.rename";
/// `track.delete` — delete a track; destructive, requires
/// `"confirm": true` ([`DeleteParams`] -> `MutationAck`).
pub const DELETE: &str = "track.delete";
/// `track.add_instrument` — set a built-in instrument by stable plugin
/// id ([`AddPluginParams`] -> [`AddPluginResult`]).
pub const ADD_INSTRUMENT: &str = "track.add_instrument";
/// `track.add_effect` — append a built-in effect to the insert chain
/// ([`AddPluginParams`] -> [`AddPluginResult`]).
pub const ADD_EFFECT: &str = "track.add_effect";
/// `track.remove_effect` — take an effect off the insert chain
/// ([`RemoveEffectParams`] -> `MutationAck`).
pub const REMOVE_EFFECT: &str = "track.remove_effect";
/// `track.move_effect` — reorder the insert chain
/// ([`MoveEffectParams`] -> `MutationAck`).
pub const MOVE_EFFECT: &str = "track.move_effect";
/// `track.set_output` — route a track to master or into a bus
/// ([`SetOutputParams`] -> `MutationAck`).
pub const SET_OUTPUT: &str = "track.set_output";
/// `track.add_send` — tap a track into a return bus
/// ([`AddSendParams`] -> [`AddSendResult`]).
pub const ADD_SEND: &str = "track.add_send";
/// `track.set_send` — edit an existing send ([`SetSendParams`]).
pub const SET_SEND: &str = "track.set_send";
/// `track.remove_send` — delete a send ([`RemoveSendParams`]).
pub const REMOVE_SEND: &str = "track.remove_send";
/// `track.plugin_params` — a track's plugins and their parameters,
/// read-only ([`PluginParamsParams`] -> [`PluginParamsView`]).
pub const PLUGIN_PARAMS: &str = "track.plugin_params";
/// `track.set_plugin_param` — set one plugin parameter
/// ([`SetPluginParamParams`] -> `MutationAck`).
pub const SET_PLUGIN_PARAM: &str = "track.set_plugin_param";
/// `track.set_sidechain` — route another track's or bus's audio into a
/// plugin's external sidechain key ([`SetSidechainParams`] ->
/// `MutationAck`).
pub const SET_SIDECHAIN: &str = "track.set_sidechain";
/// `track.clear_sidechain` — remove a plugin's key route
/// ([`ClearSidechainParams`] -> `MutationAck`).
pub const CLEAR_SIDECHAIN: &str = "track.clear_sidechain";

/// All `track.*` method names.
pub const METHODS: &[&str] = &[
    ADD,
    RENAME,
    DELETE,
    ADD_INSTRUMENT,
    ADD_EFFECT,
    REMOVE_EFFECT,
    MOVE_EFFECT,
    SET_OUTPUT,
    ADD_SEND,
    SET_SEND,
    REMOVE_SEND,
    PLUGIN_PARAMS,
    SET_PLUGIN_PARAM,
    SET_SIDECHAIN,
    CLEAR_SIDECHAIN,
];

/// Params for `track.set_sidechain`.
///
/// The plugin is addressed exactly as `track.set_plugin_param` addresses
/// it — `track_id` plus an optional `plugin_id` / `occurrence` — and the
/// key source is named by `source_track_id` **or** `source_bus_id`,
/// exactly one of which must be given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetSidechainParams {
    /// The track hosting the plugin whose key is being routed.
    pub track_id: TrackId,
    /// CLAP id of the plugin to address; omitted targets the track's
    /// instrument.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// Feed the key from this track's audio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_track_id: Option<TrackId>,
    /// Feed the key from this bus's audio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_bus_id: Option<TrackId>,
    /// A disabled route keeps its configuration but delivers no key, so
    /// the plugin falls back to keying off its own input. Defaults true.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Params for `track.clear_sidechain`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ClearSidechainParams {
    pub track_id: TrackId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

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
    /// Stable plugin id from `plugins.catalog`, e.g.
    /// `"com.resonance.wavetable"`.
    pub plugin_id: String,
}

/// Result of `track.add_effect` / `track.add_instrument`: a handle on
/// the plugin that was just created.
///
/// `(plugin_id, occurrence)` is exactly what `track.set_plugin_param`,
/// `track.plugin_params` and `track.remove_effect` take, so the caller
/// can configure or undo what it just added without re-reading the chain
/// and guessing which instance is the new one. Adding the same plugin
/// twice gives `occurrence` 0 then 1.
///
/// The plugin is visible to `track.plugin_params` in the same update
/// cycle as this reply — no engine round-trip is needed to see that it
/// exists. Its PARAMETER LIST, however, arrives with the engine's echo,
/// so a `track.plugin_params` read in the same tick may show an empty
/// `params` array; `track.set_plugin_param` says so explicitly rather
/// than claiming the plugin has no such parameter.
///
/// Additive: the previous reply was an object carrying only `revision`,
/// so a client reading just that field keeps working.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddPluginResult {
    /// The CLAP id that was added — echoed back so a reply is
    /// self-describing.
    pub plugin_id: String,
    /// 0-based index of this instance among the track's copies of
    /// `plugin_id`. Pass as `occurrence` to address it.
    pub occurrence: u32,
    /// 0-based position in the track's insert chain, the instrument
    /// included, at the moment of the add. Slots renumber when anything
    /// is removed or moved, so `(plugin_id, occurrence)` is the stabler
    /// handle.
    pub slot: u32,
    pub revision: u64,
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

// ---------------------------------------------------------------------------
// Aux sends (ba doc #273, todo #1229)
// ---------------------------------------------------------------------------

/// Lowest send level the engine accepts, in dB (it clamps to this).
pub const SEND_LEVEL_DB_MIN: f32 = -120.0;
/// Highest send level the engine accepts, in dB.
pub const SEND_LEVEL_DB_MAX: f32 = 24.0;

/// Params for `track.add_send` — an extra tap from a track into a return
/// bus, independent of where the track's main output goes.
///
/// This is how several tracks share ONE reverb (or delay): each sends
/// some of its signal to a common return, which puts them in the same
/// room. A reverb inserted per track puts every instrument in a
/// different building and costs far more CPU.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddSendParams {
    pub track_id: TrackId,
    /// The destination bus (from `bus.create`, or a `kind: "bus"` entry
    /// in `song.summary`). It is flagged as a RETURN bus automatically.
    pub to_bus: TrackId,
    /// Send gain in dB; `0` taps the source at unity. Defaults to `0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_db: Option<f32>,
    /// `true` taps before the track's own fader (the send level then
    /// ignores the fader), `false` — the default, and what you almost
    /// always want for reverb — taps after it, so moving the track down
    /// takes its reverb with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_fader: Option<bool>,
}

/// Result of `track.add_send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddSendResult {
    /// Address this send in `track.set_send` / `track.remove_send`; it
    /// is also reported in `song.tracks`.
    pub send_id: SendId,
    pub revision: u64,
}

/// Params for `track.set_send`: change one or more properties of an
/// existing send. Omitted fields keep their current value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetSendParams {
    pub send_id: SendId,
    /// New send gain in dB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_db: Option<f32>,
    /// Move the tap before (`true`) or after (`false`) the fader.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_fader: Option<bool>,
    /// Silence the send without losing its routing and level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Re-route the send into a different return bus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_bus: Option<TrackId>,
}

/// Params for `track.remove_send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RemoveSendParams {
    pub send_id: SendId,
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

/// Params for `track.move_effect`: change WHERE an effect sits in the
/// chain, addressed exactly like [`RemoveEffectParams`] — **either**
/// `slot` **or** `plugin_id` (+ `occurrence`), never both and never
/// neither.
///
/// Order is audible. A compressor after an EQ reacts to the EQ'd signal;
/// the same compressor before it does not, and the two are different
/// sounds, not different spellings of one. A limiter belongs last,
/// because anything after it can push the signal back over the ceiling
/// it was there to hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MoveEffectParams {
    pub track_id: TrackId,
    /// 0-based chain position of the effect to move, as reported by
    /// [`PluginParamsEntry::slot`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// CLAP id of the effect to move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based; defaults to the first.
    /// Only meaningful together with `plugin_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// Destination chain position, 0-based. A value past the end of the
    /// chain clamps to the last slot rather than failing. Moving an
    /// effect to the slot it already occupies is an accepted no-op.
    pub to_slot: u32,
}

/// One plugin parameter.
///
/// `min`/`max`/`default`/`value` are f64 on the wire, but plugins
/// declare their ranges in **f32**, so a bound with no exact binary
/// representation arrives with a long tail of digits: a minimum the
/// plugin calls `0.1` is reported as `0.10000000149011612`. That is not
/// a rounding bug to work around — see [`SetPluginParamParams::value`]
/// for what `track.set_plugin_param` accepts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginParamView {
    /// Stable CLAP parameter id. Either this (as a decimal string) or
    /// `name` addresses the parameter in `track.set_plugin_param`.
    pub id: u32,
    pub name: String,
    pub value: f64,
    /// Lowest accepted value, as an f64 rendering of the plugin's f32
    /// declaration — so it may read `0.10000000149011612` where the
    /// plugin means `0.1`. Sending the tidy decimal is fine: it rounds
    /// onto the bound and is clamped, not rejected.
    pub min: f64,
    /// Highest accepted value, with the same f32-widening caveat as
    /// [`min`](Self::min).
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
    /// New value. Must lie within the parameter's `min..=max`, which an
    /// out-of-range request reports back.
    ///
    /// Those bounds are f64 renderings of **f32** plugin declarations
    /// ([`PluginParamView::min`]), so a value that rounds onto a bound —
    /// `0.1` against a reported minimum of `0.10000000149011612` — is
    /// ACCEPTED and clamped to the true bound, never rejected for being
    /// a few ULPs out. Anything past that tolerance is still refused
    /// rather than clamped, so a genuinely wrong number (`-60` on a
    /// `0..1` parameter) comes back as an error instead of silently
    /// becoming something else.
    pub value: f64,
}
