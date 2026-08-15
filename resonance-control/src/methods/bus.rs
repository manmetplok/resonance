//! `bus.*` — group busses, the stage between tracks and master.
//!
//! Busses have always existed in the app and the engine; they were
//! simply unreachable from the control API, so a client could neither
//! create one nor route a track into one (ba doc #273). Busses are
//! *listed* by `song.summary` / `song.tracks` as tracks with
//! `kind: "bus"`, from a distinct id range — there is no separate list
//! method.
//!
//! [`CREATE`] returns the new id in its reply; the others return
//! [`crate::common::MutationAck`].

use crate::ids::TrackId;
use crate::methods::track::PluginParamsEntry;
use serde::{Deserialize, Serialize};

/// `bus.create` — add a group bus ([`CreateParams`] -> [`CreateResult`]).
pub const CREATE: &str = "bus.create";
/// `bus.delete` — remove a bus; destructive, requires `"confirm": true`
/// ([`DeleteParams`] -> `MutationAck`).
pub const DELETE: &str = "bus.delete";
/// `bus.set_volume` — set a bus fader in dB ([`SetVolumeParams`]).
pub const SET_VOLUME: &str = "bus.set_volume";
/// `bus.add_effect` — append an effect to a bus's insert chain
/// ([`AddEffectParams`] -> [`crate::methods::track::AddPluginResult`]).
pub const ADD_EFFECT: &str = "bus.add_effect";
/// `bus.remove_effect` — take an effect off a bus's chain
/// ([`RemoveEffectParams`] -> `MutationAck`).
pub const REMOVE_EFFECT: &str = "bus.remove_effect";
/// `bus.move_effect` — reorder a bus's chain ([`MoveEffectParams`] ->
/// `MutationAck`).
pub const MOVE_EFFECT: &str = "bus.move_effect";
/// `bus.set_fx_bypass` — bypass/unbypass a bus's whole chain
/// ([`SetFxBypassParams`] -> `MutationAck`).
pub const SET_FX_BYPASS: &str = "bus.set_fx_bypass";
/// `bus.plugin_params` — a bus's plugins and their parameters, read-only
/// ([`PluginParamsParams`] -> [`PluginParamsView`]).
pub const PLUGIN_PARAMS: &str = "bus.plugin_params";
/// `bus.set_plugin_param` — set one parameter on a bus plugin
/// ([`SetPluginParamParams`] -> `MutationAck`).
pub const SET_PLUGIN_PARAM: &str = "bus.set_plugin_param";
/// `bus.set_sidechain` — key a plugin on a bus's chain from another
/// track or bus ([`SetSidechainParams`] -> `MutationAck`).
pub const SET_SIDECHAIN: &str = "bus.set_sidechain";
/// `bus.clear_sidechain` — remove a bus plugin's key route
/// ([`ClearSidechainParams`] -> `MutationAck`).
pub const CLEAR_SIDECHAIN: &str = "bus.clear_sidechain";

/// All `bus.*` method names.
pub const METHODS: &[&str] = &[
    CREATE,
    DELETE,
    SET_VOLUME,
    ADD_EFFECT,
    REMOVE_EFFECT,
    MOVE_EFFECT,
    SET_FX_BYPASS,
    PLUGIN_PARAMS,
    SET_PLUGIN_PARAM,
    SET_SIDECHAIN,
    CLEAR_SIDECHAIN,
];

/// Params for `bus.create`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CreateParams {
    /// Display name, e.g. `"Drum Bus"`. Omitted, the app names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Result of `bus.create`. The id is allocated app-side and returned
/// immediately, so a client can route tracks into the bus in its very
/// next call rather than polling `song.summary` for it to appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CreateResult {
    /// The new bus, in the same id space `song.summary` reports busses
    /// under and `track.set_output` accepts.
    pub bus_id: TrackId,
    pub revision: u64,
}

/// Params for `bus.delete`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DeleteParams {
    pub bus_id: TrackId,
    /// Required (`true`). Tracks routed to the bus are NOT deleted —
    /// they fall back to master — but the bus's own level and effects
    /// are lost, which changes how the group sounds.
    #[serde(default)]
    pub confirm: bool,
}

/// Params for `bus.set_volume`.
///
/// Unlike a track fader, a bus level is expressed **only** in decibels
/// here: it is the unit the app stores and the unit group balance is
/// reasoned about in.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetVolumeParams {
    pub bus_id: TrackId,
    /// Decibels, 0 = unity. Same range as a track fader:
    /// [`crate::methods::mixer::VOLUME_DB_MIN`]`..=`[`crate::methods::mixer::VOLUME_DB_MAX`].
    pub volume_db: f32,
}

// ---------------------------------------------------------------------------
// The bus insert chain (ba doc #273, todo #1237)
// ---------------------------------------------------------------------------

/// Params for `bus.add_effect`. Appends, exactly like
/// `track.add_effect`: calling it twice with the same id puts two
/// instances on the bus.
///
/// A bus chain processes the SUM of everything routed into the bus, so
/// it is the only place a peak created by several sources hitting at
/// once — kick plus snare plus crash — can be controlled. Inserting the
/// same compressor on each contributing track cannot do it: none of them
/// individually is loud enough to trigger it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddEffectParams {
    pub bus_id: TrackId,
    /// The effect's stable CLAP id, e.g. `"com.resonance.compressor"`.
    /// Instrument plugins are refused — a bus chain takes effects only,
    /// because it is handed audio, not notes.
    pub plugin_id: String,
}

/// Params for `bus.remove_effect`: address the plugin **either** by
/// `slot` **or** by `plugin_id` (+ `occurrence`). Both forms together,
/// or neither, is `invalid_params` — identical rules to
/// `track.remove_effect`, for the identical reason: with `bus.add_effect`
/// appending a fresh instance on every call, guessing which copy was
/// meant is how the wrong processor comes off a whole group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RemoveEffectParams {
    pub bus_id: TrackId,
    /// 0-based chain position, as reported by `bus.plugin_params`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// The effect's CLAP id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based; defaults to the first.
    /// Only meaningful together with `plugin_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

/// Params for `bus.move_effect`: same addressing as
/// [`RemoveEffectParams`], plus where it should end up.
///
/// Order is audible on a bus for the same reason it is on a track — the
/// chain runs front to back — and it matters more here, because
/// everything routed into the bus goes through it. A bus chain has no
/// structural first slot (unlike a track's instrument), so any order is
/// valid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MoveEffectParams {
    pub bus_id: TrackId,
    /// 0-based chain position of the effect to move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// CLAP id of the effect to move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based; defaults to the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// Destination chain position, 0-based. Past the end clamps to the
    /// last slot; moving an effect to where it already sits is an
    /// accepted no-op.
    pub to_slot: u32,
}

/// Params for `bus.set_fx_bypass`. Idempotent: this SETS the state
/// rather than toggling it, so a client that lost track of the current
/// value — or that retried a request it never saw the answer to — cannot
/// flip the group's processing back on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetFxBypassParams {
    pub bus_id: TrackId,
    /// `true` bypasses every plugin on this bus's chain (the group
    /// passes through unprocessed); `false` re-engages them.
    pub bypassed: bool,
}

/// Params for `bus.plugin_params` — read a bus's chain the way
/// `track.plugin_params` reads a track's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginParamsParams {
    pub bus_id: TrackId,
    /// CLAP id of a plugin on this bus. Omitted returns every plugin on
    /// it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance to address when the bus carries `plugin_id` more
    /// than once; 0-based, defaults to the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

/// Result of `bus.plugin_params`.
///
/// The entries are the SAME [`PluginParamsEntry`] shape
/// `track.plugin_params` returns, so a client reads a bus chain with the
/// code it already has for a track chain. Every entry is
/// `kind: "effect"` — a bus has no instrument slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginParamsView {
    pub bus_id: TrackId,
    pub plugins: Vec<PluginParamsEntry>,
    pub revision: u64,
}

/// Params for `bus.set_plugin_param`.
///
/// This is a separate method rather than an overload of
/// `track.set_plugin_param` on purpose: that method's params are keyed
/// on `track_id`, and passing a bus id in a field called `track_id` is
/// the exact class of naming trap the `track.plugins` ->
/// `plugins.catalog` rename removed. `resonance-mcp` also publishes one
/// tool per method, so an overload would have had to smuggle two
/// meanings through one field an agent reads once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetPluginParamParams {
    pub bus_id: TrackId,
    /// CLAP id of the plugin on this bus. Omitted targets the bus's
    /// FIRST plugin, which is unambiguous only on a one-effect chain —
    /// name it whenever the bus carries more than one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based; defaults to the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// The parameter, by name (case-insensitive) or by its numeric id as
    /// a string. Names come from `bus.plugin_params`.
    pub param: String,
    /// New value. Same bounds handling as
    /// [`crate::methods::track::SetPluginParamParams::value`]: a value
    /// that rounds onto an f32-declared bound is accepted and clamped,
    /// anything past that tolerance is rejected.
    pub value: f64,
}

// ---------------------------------------------------------------------------
// Sidechain (key) routing onto a bus chain (ba doc #275 P4, todo #1311)
// ---------------------------------------------------------------------------

/// Params for `bus.set_sidechain` — feed another track's or bus's audio
/// into the key input of a plugin **on a bus chain**.
///
/// This is the arm the control API was missing. `track.set_sidechain` is
/// keyed on `track_id`, so a compressor on the bass *bus* — the single
/// most common place a keyed ducker actually lives, because the point is
/// to duck a whole group from one hit — could not be keyed at all. The
/// engine never had that limitation: its route table is keyed by plugin
/// instance, and the mixer connects a key port wherever the plugin sits.
///
/// A separate method rather than an overload of `track.set_sidechain`,
/// for the reason [`SetPluginParamParams`] gives: `resonance-mcp`
/// publishes one tool per method, so an overload would smuggle two
/// meanings through one field an agent reads once.
///
/// Addressing and the two safety rules are otherwise identical to
/// [`crate::methods::track::SetSidechainParams`]: an omitted `plugin_id`
/// targets the first plugin on the bus that declares a key port, and a
/// route onto a plugin with no key port is refused rather than stored
/// inertly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetSidechainParams {
    /// The bus hosting the plugin whose key is being routed.
    pub bus_id: TrackId,
    /// CLAP id of the plugin to address; omitted targets the first
    /// plugin on the bus that declares a key port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based; defaults to the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// Feed the key from this track's audio. A sub-track (one tap of a
    /// multi-output instrument, e.g. the kick of a kit) is a valid
    /// source and is captured post-FX, pre-fader.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_track_id: Option<TrackId>,
    /// Feed the key from this bus's audio. A bus may legally key a
    /// plugin on itself — the key is delivered one block late by
    /// construction, so the route cannot feed back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_bus_id: Option<TrackId>,
    /// A disabled route keeps its configuration but delivers no key, so
    /// the plugin falls back to keying off its own input. Defaults true.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Params for `bus.clear_sidechain`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ClearSidechainParams {
    pub bus_id: TrackId,
    /// Omitted targets the same plugin `bus.set_sidechain` would: the
    /// first on the bus with a key port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

fn default_true() -> bool {
    true
}
