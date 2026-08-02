//! `master.*` — the final summing stage, after every track and bus.
//!
//! The master bus has always existed in the app (volume, an insert
//! chain, an FX bypass); it was simply unreachable from the control API,
//! which is why a mix driven entirely over MCP could be balanced
//! correctly and still have nowhere to put a limiter (ba doc #273).
//!
//! [`SUMMARY`] is read-only but needs an open project. [`SET_VOLUME`]
//! mutates and returns [`crate::common::MutationAck`].

use serde::{Deserialize, Serialize};

/// `master.summary` — master volume, FX bypass and insert chain
/// ([`MasterSummary`]). No params. Read-only.
pub const SUMMARY: &str = "master.summary";
/// `master.set_volume` — set the master fader ([`SetMasterVolumeParams`]).
pub const SET_VOLUME: &str = "master.set_volume";
/// `master.add_effect` — append an effect to the master insert chain
/// ([`AddEffectParams`]).
pub const ADD_EFFECT: &str = "master.add_effect";
/// `master.remove_effect` — take an effect off the master chain
/// ([`RemoveEffectParams`]).
pub const REMOVE_EFFECT: &str = "master.remove_effect";
/// `master.set_fx_bypass` — bypass/unbypass the whole master chain
/// ([`SetFxBypassParams`]).
pub const SET_FX_BYPASS: &str = "master.set_fx_bypass";

/// All `master.*` method names.
pub const METHODS: &[&str] = &[
    SUMMARY,
    SET_VOLUME,
    ADD_EFFECT,
    REMOVE_EFFECT,
    SET_FX_BYPASS,
];

/// Result of `master.summary`.
///
/// The plugin chain is reported here as well as by any future
/// `master.*_effect` method, because a client has to see what is already
/// on the master before it can decide what to add.
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
