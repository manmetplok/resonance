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
use serde::{Deserialize, Serialize};

/// `bus.create` — add a group bus ([`CreateParams`] -> [`CreateResult`]).
pub const CREATE: &str = "bus.create";
/// `bus.delete` — remove a bus; destructive, requires `"confirm": true`
/// ([`DeleteParams`] -> `MutationAck`).
pub const DELETE: &str = "bus.delete";
/// `bus.set_volume` — set a bus fader in dB ([`SetVolumeParams`]).
pub const SET_VOLUME: &str = "bus.set_volume";

/// All `bus.*` method names.
pub const METHODS: &[&str] = &[CREATE, DELETE, SET_VOLUME];

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
