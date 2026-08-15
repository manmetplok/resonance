//! `master.*` — the final summing stage, after every track and bus.
//!
//! The master bus has always existed in the app (volume, an insert
//! chain, an FX bypass); it was simply unreachable from the control API,
//! which is why a mix driven entirely over MCP could be balanced
//! correctly and still have nowhere to put a limiter (ba doc #273).
//!
//! [`SUMMARY`] and [`PLUGIN_PARAMS`] are read-only but need an open
//! project. The rest mutate and return
//! [`crate::common::MutationAck`], except [`ADD_EFFECT`], which returns
//! the new plugin's slot.

use crate::methods::track::PluginParamsEntry;
use serde::{Deserialize, Serialize};

/// `master.summary` — master volume, FX bypass and insert chain
/// ([`MasterSummary`]). No params. Read-only.
pub const SUMMARY: &str = "master.summary";
/// `master.set_volume` — set the master fader ([`SetMasterVolumeParams`]).
pub const SET_VOLUME: &str = "master.set_volume";
/// `master.add_effect` — append an effect to the master insert chain
/// ([`AddEffectParams`] -> [`crate::methods::track::AddPluginResult`]).
pub const ADD_EFFECT: &str = "master.add_effect";
/// `master.remove_effect` — take an effect off the master chain
/// ([`RemoveEffectParams`]).
pub const REMOVE_EFFECT: &str = "master.remove_effect";
/// `master.move_effect` — reorder the master chain ([`MoveEffectParams`]
/// -> `MutationAck`).
pub const MOVE_EFFECT: &str = "master.move_effect";
/// `master.set_fx_bypass` — bypass/unbypass the whole master chain
/// ([`SetFxBypassParams`]).
pub const SET_FX_BYPASS: &str = "master.set_fx_bypass";
/// `master.plugin_params` — the master chain's plugins and their
/// parameters, read-only ([`PluginParamsParams`] -> [`PluginParamsView`]).
pub const PLUGIN_PARAMS: &str = "master.plugin_params";
/// `master.set_plugin_param` — set one parameter on a master plugin
/// ([`SetPluginParamParams`] -> `MutationAck`).
pub const SET_PLUGIN_PARAM: &str = "master.set_plugin_param";

/// All `master.*` method names.
pub const METHODS: &[&str] = &[
    SUMMARY,
    SET_VOLUME,
    ADD_EFFECT,
    REMOVE_EFFECT,
    MOVE_EFFECT,
    SET_FX_BYPASS,
    PLUGIN_PARAMS,
    SET_PLUGIN_PARAM,
];

/// Result of `master.summary`.
///
/// The plugin chain is reported here as well as by any future
/// `master.*_effect` method, because a client has to see what is already
/// on the master before it can decide what to add. Identity only,
/// though — for a plugin's parameters and their ranges, read
/// [`PLUGIN_PARAMS`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MasterSummary {
    /// Linear master gain (1.0 = unity).
    pub volume: f32,
    /// The same fader in decibels (0 dB = unity) — what the app stores
    /// and what the mixer's master strip shows.
    pub volume_db: f32,
    /// True while every plugin on the master chain is bypassed.
    pub fx_bypassed: bool,
    /// The master insert chain in processing order. Empty means nothing
    /// is inserted on the master at all.
    #[serde(default)]
    pub plugins: Vec<MasterPluginEntry>,
}

/// One plugin on the master insert chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MasterPluginEntry {
    /// 0-based position in the master chain; matches processing order.
    pub slot: u32,
    /// The plugin's stable CLAP id, e.g. `"com.resonance.mastering"`.
    pub plugin_id: String,
    /// Human-readable plugin name.
    pub name: String,
}

/// Params for `master.set_volume`: give **exactly one** of `volume`
/// (linear gain) or `volume_db` (decibels). Supplying both, or neither,
/// is `invalid_params` — guessing which one the caller meant is how a
/// master fader ends up 20 dB off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetMasterVolumeParams {
    /// Linear gain (1.0 = unity, 0.0 = silence).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f32>,
    /// Decibels (0 = unity). Same range as a track fader:
    /// [`crate::methods::mixer::VOLUME_DB_MIN`]`..=`[`crate::methods::mixer::VOLUME_DB_MAX`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_db: Option<f32>,
}

/// Params for `master.add_effect`. Appends, exactly like
/// `track.add_effect`: calling it twice with the same id puts two
/// instances on the master.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddEffectParams {
    /// The effect's stable CLAP id, e.g. `"com.resonance.mastering"`.
    /// Instrument plugins are refused — the master chain processes an
    /// already-summed mix and has no notes to play.
    pub plugin_id: String,
}

/// Params for `master.remove_effect`: address the plugin **either** by
/// `slot` **or** by `plugin_id` (+ `occurrence` when the same effect is
/// on the master more than once). Giving both forms, or neither, is
/// `invalid_params` — a guess here removes the wrong processor from the
/// whole mix.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RemoveEffectParams {
    /// 0-based chain position, as reported by `master.summary`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// The effect's CLAP id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based. Defaults to `0`; only
    /// meaningful together with `plugin_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

/// Params for `master.move_effect`: same addressing as
/// [`RemoveEffectParams`], plus where the effect should end up.
///
/// Order is audible — the chain runs front to back over the finished
/// mix — and on the master it decides whether the chain does its job at
/// all: a limiter holding a ceiling has to be LAST, because anything
/// after it can push the sum back over that ceiling. The master chain
/// has no structural first slot (unlike a track's instrument), so any
/// order is valid.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MoveEffectParams {
    /// 0-based chain position of the effect to move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// CLAP id of the effect to move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based; defaults to the first.
    /// Only meaningful together with `plugin_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// Destination chain position, 0-based. Past the end clamps to the
    /// last slot; moving an effect to where it already sits is an
    /// accepted no-op.
    pub to_slot: u32,
}

/// Params for `master.set_fx_bypass`. Idempotent: this SETS the state
/// rather than toggling it, so a client that lost track of the current
/// value cannot flip it the wrong way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetFxBypassParams {
    /// `true` bypasses every plugin on the master chain (the mix passes
    /// through unprocessed); `false` re-engages them.
    pub bypassed: bool,
}

// ---------------------------------------------------------------------------
// Master plugin parameters (ba doc #273 follow-up)
// ---------------------------------------------------------------------------
//
// Without these the master chain is write-only: `master.add_effect`
// loads a limiter and it then sits at its defaults forever, which is why
// a mix driven over the control API could reach the master and still not
// reach a release level. Master is a singleton, so these are the `bus.*`
// methods with `bus_id` dropped — same addressing, same errors, same
// acks.

/// Params for `master.plugin_params` — read the master chain the way
/// `bus.plugin_params` reads a bus's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginParamsParams {
    /// CLAP id of a plugin on the master. Omitted returns every plugin
    /// on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance to address when the master carries `plugin_id`
    /// more than once; 0-based, defaults to the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

/// Result of `master.plugin_params`.
///
/// The entries are the SAME [`PluginParamsEntry`] shape
/// `track.plugin_params` and `bus.plugin_params` return — including
/// `slot`, the 0-based chain position that IS processing order — so a
/// client reads the master chain with the code it already has. Every
/// entry is `kind: "effect"`: the master has no instrument slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginParamsView {
    pub plugins: Vec<PluginParamsEntry>,
    pub revision: u64,
}

/// Params for `master.set_plugin_param`.
///
/// The counterpart to `bus.set_plugin_param`, minus the id: this is what
/// turns a limiter on the master from "present at its default patch"
/// into one that actually holds a ceiling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetPluginParamParams {
    /// CLAP id of the plugin on the master. Omitted targets the FIRST
    /// plugin on the chain, which is unambiguous only on a one-effect
    /// master — name it whenever the master carries more than one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based; defaults to the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// The parameter, by name (case-insensitive) or by its numeric id as
    /// a string. Names come from [`PLUGIN_PARAMS`].
    pub param: String,
    /// New value — a number, or a choice label from
    /// `master.plugin_params`. Same handling as
    /// [`crate::methods::track::SetPluginParamParams::value`]: a value
    /// that rounds onto an f32-declared bound is accepted and clamped,
    /// anything past that tolerance is rejected with the range, and a
    /// label resolves to the step it names.
    pub value: crate::methods::track::ParamValue,
}
