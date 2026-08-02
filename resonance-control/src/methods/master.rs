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

/// All `master.*` method names.
pub const METHODS: &[&str] = &[SUMMARY, SET_VOLUME];

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
