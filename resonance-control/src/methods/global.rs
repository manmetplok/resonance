//! `global.*` — the song-wide tempo and time-signature tracks (ba doc
//! #286, epic #205).
//!
//! The app has always had a global-tracks shelf carrying a list of tempo
//! changes and a list of meter changes, each anchored to a bar. Nothing
//! on the control surface could see it: `transport.set_tempo` and
//! `transport.set_time_signature` rewrite the FIRST event of each list
//! and leave every later change standing, and `song.summary` reports the
//! values at the PLAYHEAD, which on a song with changes is not a property
//! of the song at all. `global.*` is the honest view of both lists, and
//! (in the slices that follow) the way to edit them.
//!
//! # Addressing: by bar, never by index
//!
//! Both lists re-sort by bar on every mutation, so a list index handed to
//! a client is stale the moment anything is inserted before it. Every
//! `global.*` method therefore addresses an event by the BAR it sits on;
//! bars are unique within each list (adding at an occupied bar overwrites
//! that event rather than duplicating it). Handlers resolve bar -> index
//! internally, immediately before dispatch, and never hand one out.
//!
//! # Bars are 1-based on the wire
//!
//! As everywhere on this surface (see
//! [`arrangement::InsertBarsParams::at_bar`](crate::methods::arrangement::InsertBarsParams::at_bar)),
//! `bar` is 1-based here — bar 1 is the start of the song. App state
//! stores these events 0-based; the conversion happens once, at the
//! handler boundary.
//!
//! # Bar 1 is special
//!
//! Each list always has an event at bar 1: the song's initial tempo and
//! its initial meter. It cannot be removed (only edited, which is what
//! `transport.set_tempo` / `transport.set_time_signature` do), so both
//! lists are always non-empty and a song with no changes reports exactly
//! one entry in each.

use serde::{Deserialize, Serialize};

/// `global.list_events` — read both global tracks: every tempo change and
/// every meter change in the song, with the bar each takes effect at. No
/// params; returns [`GlobalEvents`]. Read-only.
///
/// `song.summary` carries these same two lists (ba todo #1381), so a
/// client that already reads the summary does not need this call to
/// learn whether the song changes tempo or meter. What it must not do is
/// trust the summary's `tempo_bpm` / `time_signature`: those are the
/// values at the playhead and say nothing about the rest of the song.
pub const LIST_EVENTS: &str = "global.list_events";

/// `global.add_tempo_event` — put a tempo change on the tempo track at a
/// bar ([`AddTempoEventParams`] -> [`MutationAck`](crate::MutationAck)).
///
/// **Upsert by bar.** A bar carries at most one tempo event, so adding at
/// a bar that already has one REPLACES its BPM rather than stacking a
/// second event there. Calling twice with different values therefore
/// leaves the second value and one event, not two — which is also what
/// makes the call safe to retry.
///
/// Bar 1 is the song's initial tempo; adding there rewrites it, exactly
/// as `transport.set_tempo` does.
pub const ADD_TEMPO_EVENT: &str = "global.add_tempo_event";

/// `global.add_signature_event` — put a meter change on the signature
/// track at a bar ([`AddSignatureEventParams`] ->
/// [`MutationAck`](crate::MutationAck)).
///
/// **Upsert by bar**, on the same rule as [`ADD_TEMPO_EVENT`]: one meter
/// per bar, a second add at the same bar replaces the first.
///
/// Bar 1 is the song's initial meter; adding there rewrites it, which is
/// all `transport.set_time_signature` can do. Every LATER meter change
/// needs this method.
pub const ADD_SIGNATURE_EVENT: &str = "global.add_signature_event";

/// All `global.*` method names.
pub const METHODS: &[&str] = &[LIST_EVENTS, ADD_TEMPO_EVENT, ADD_SIGNATURE_EVENT];

/// Params for [`ADD_TEMPO_EVENT`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddTempoEventParams {
    /// 1-based bar the new tempo takes effect at. An existing tempo
    /// event at this bar is overwritten, not duplicated. Bar 0 is
    /// rejected; bar 1 rewrites the song's initial tempo.
    pub bar: u32,
    /// Beats per minute from this bar until the next tempo event.
    /// Rejected outside 20..=300 rather than clamped, so a caller that
    /// asked for something impossible learns it instead of silently
    /// getting a different tempo.
    pub bpm: f32,
}

/// Params for [`ADD_SIGNATURE_EVENT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddSignatureEventParams {
    /// 1-based bar the new meter takes effect at. An existing meter
    /// event at this bar is overwritten, not duplicated. Bar 0 is
    /// rejected; bar 1 rewrites the song's initial meter.
    pub bar: u32,
    /// Beats per bar (the top number), 1..=32.
    pub numerator: u8,
    /// Note value that gets the beat (the bottom number): a power of two
    /// in 1..=32, e.g. 8 for 7/8. Resolved, not an exponent — an
    /// out-of-range or non-power-of-two value is rejected.
    pub denominator: u8,
}

/// One tempo change on the global tempo track.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TempoEventView {
    /// 1-based bar this tempo takes effect at, and the address every
    /// `global.*` tempo method uses. Bar 1 is the song's initial tempo
    /// and is always present.
    pub bar: u32,
    /// Beats per minute from this bar until the next tempo event (or the
    /// end of the song).
    pub bpm: f32,
}

/// One time-signature change on the global signature track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SignatureEventView {
    /// 1-based bar this meter takes effect at, and the address every
    /// `global.*` signature method uses. Bar 1 is the song's initial
    /// meter and is always present.
    pub bar: u32,
    /// Beats per bar (the top number), 1..=32.
    pub numerator: u8,
    /// Note value that gets the beat (the bottom number): a power of two
    /// in 1..=32, e.g. 8 for 7/8. Resolved, not an exponent.
    pub denominator: u8,
}

/// Result of `global.list_events`: both global tracks in full.
///
/// A song with no changes reports one entry in each list — the bar-1
/// initial events — so "does this song change tempo or meter?" is
/// answered by the list lengths without a second call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct GlobalEvents {
    /// Every tempo change, sorted by bar, starting at bar 1.
    pub tempo_events: Vec<TempoEventView>,
    /// Every time-signature change, sorted by bar, starting at bar 1.
    pub signature_events: Vec<SignatureEventView>,
    /// The app's edit-revision counter at the time of the read, so a
    /// caller can tell whether the song changed under it.
    pub revision: u64,
}
