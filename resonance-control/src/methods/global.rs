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
//! one entry in each. It cannot be MOVED either — it is what "the start
//! of the song" means — so [`EDIT_TEMPO_EVENT`]'s `new_bar` is refused
//! on it.
//!
//! # Add creates, edit changes
//!
//! `add_*` upserts: it writes the event whether or not one was there.
//! `edit_*` addresses one that already exists and refuses an empty bar,
//! because the two possible readings of "edit a bar with nothing on it"
//! — create it, or tell me I am wrong about the song — lead to very
//! different songs, and only one of them is recoverable.

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

/// `global.edit_tempo_event` — change the tempo event that already sits
/// on a bar ([`EditTempoEventParams`] -> [`MutationAck`](crate::MutationAck)).
///
/// **Addresses an existing event; it does not create one.** `bar` names
/// the event to change, and a bar with no tempo event there is an
/// `invalid_params` refusal rather than an upsert —
/// [`ADD_TEMPO_EVENT`] is the method that creates. Omitted fields keep
/// their current value, so `{bar: 33, bpm: 96}` retunes the bridge
/// without touching where it starts.
///
/// This is the one method on either track that can MOVE an event:
/// `new_bar` relocates it, keeping the list sorted. Bar 1 is the
/// exception — the song's initial tempo may be retuned but not
/// relocated, and asking is refused rather than ignored.
pub const EDIT_TEMPO_EVENT: &str = "global.edit_tempo_event";

/// `global.edit_signature_event` — change the meter event that already
/// sits on a bar ([`EditSignatureEventParams`] ->
/// [`MutationAck`](crate::MutationAck)).
///
/// **Addresses an existing event; it does not create one**, on the same
/// rule as [`EDIT_TEMPO_EVENT`]: an empty bar is refused, and
/// [`ADD_SIGNATURE_EVENT`] is the method that creates. Omitted fields
/// keep their current value, so `{bar: 17, numerator: 5}` turns 7/8 into
/// 5/8.
///
/// Unlike [`EDIT_TEMPO_EVENT`] it has no `new_bar`: a meter event's bar
/// is not editable in place. Put the meter change at the bar you want
/// with [`ADD_SIGNATURE_EVENT`] and drop the old one.
pub const EDIT_SIGNATURE_EVENT: &str = "global.edit_signature_event";

/// All `global.*` method names.
pub const METHODS: &[&str] = &[
    LIST_EVENTS,
    ADD_TEMPO_EVENT,
    ADD_SIGNATURE_EVENT,
    EDIT_TEMPO_EVENT,
    EDIT_SIGNATURE_EVENT,
];

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

/// Params for [`EDIT_TEMPO_EVENT`].
///
/// At least one of `bpm` / `new_bar` must be present: a call that changes
/// nothing is refused rather than acknowledged, so a caller that omitted
/// the field it meant to send finds out instead of reading back an
/// unchanged track and wondering.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct EditTempoEventParams {
    /// 1-based bar of the tempo event to change. There must already be
    /// one exactly there — an empty bar is rejected, not filled in
    /// (that is [`ADD_TEMPO_EVENT`]'s job).
    pub bar: u32,
    /// New beats per minute, or absent to keep the current tempo.
    /// Rejected outside 20..=300 rather than clamped, matching
    /// [`AddTempoEventParams::bpm`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f32>,
    /// 1-based bar to move the event to, or absent to leave it where it
    /// is. The list stays sorted afterwards.
    ///
    /// Rejected when it already carries a different tempo event (one bar
    /// holds at most one), and rejected when `bar` is 1: the song's
    /// initial tempo is anchored to the start of the song and can only
    /// be retuned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_bar: Option<u32>,
}

/// Params for [`EDIT_SIGNATURE_EVENT`].
///
/// At least one of `numerator` / `denominator` must be present, for the
/// same reason as [`EditTempoEventParams`]. There is deliberately no
/// `new_bar`: see [`EDIT_SIGNATURE_EVENT`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct EditSignatureEventParams {
    /// 1-based bar of the meter event to change. There must already be
    /// one exactly there — an empty bar is rejected, not filled in
    /// (that is [`ADD_SIGNATURE_EVENT`]'s job).
    pub bar: u32,
    /// New beats per bar (the top number), 1..=32, or absent to keep the
    /// current one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numerator: Option<u8>,
    /// New note value that gets the beat (the bottom number): a power of
    /// two in 1..=32, resolved rather than an exponent. Absent keeps the
    /// current one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denominator: Option<u8>,
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
