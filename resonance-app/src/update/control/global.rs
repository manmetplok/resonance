//! `global.*` control handlers (ba doc #286, epic #205): the song-wide
//! tempo and time-signature tracks.
//!
//! # Where the events live
//!
//! `app.tempo_events` / `app.signature_events` are the same two vectors
//! the global-tracks shelf draws, feeding `TempoMap` through
//! `rebuild_and_send_tempo`. This file is the wire boundary onto them.
//!
//! # Addressing: bar in, index out — never the reverse
//!
//! Both vectors re-sort by bar on every mutation, so an index is stale
//! the moment anything is inserted before it and must never leave this
//! process. Clients name the BAR; [`tempo_event_index`] /
//! [`signature_event_index`] resolve it to an index immediately before
//! dispatch, and nothing holds one across a call.
//!
//! Wire bars are 1-based (as everywhere on this surface); the app stores
//! these events 0-based. [`state_bar`] and [`wire_bar`] are the only two
//! places that conversion happens.
//!
//! # Undo, and why every mutator must route through `update()`
//!
//! Global-track edits are ALREADY undoable — not here, but in the
//! central classifier: `undo::classify` maps `Message::GlobalTrack(_)` to
//! `UndoAction::Record`, and the undo snapshot serializes both event
//! lists. That only works for messages that actually go through
//! `Resonance::update`, so the rule for every mutating `global.*` handler
//! (slices #1382-#1384) is: synthesize a `GlobalTrackMessage` and send it
//! via [`super::run_via_update`]. Writing `app.tempo_events` directly
//! would produce an edit the user cannot undo, and no test that skips
//! undo would notice.
//!
//! # Gating
//!
//! `global.list_events` mutates nothing, but it sits BELOW the shared
//! mutation gate in `super::execute` on purpose, exactly like
//! `master.summary` and `pool.list`: it describes the OPEN project's
//! tempo map, so with nothing open the honest answer is `busy` rather
//! than a made-up 120 BPM 4/4 that reads like a real song.

use crate::message::Message;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::global::{
    self as proto, GlobalEvents, SignatureEventView, TempoEventView,
};
use resonance_control::{Request, Response, RpcError};

/// Handle a `global.*` request, or `None` when `method` belongs to
/// another namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let handled = match request.method.as_str() {
        proto::LIST_EVENTS => list_events(app, request),
        _ => return None,
    };
    Some(handled)
}

/// `global.list_events` — both global tracks, in bar order.
fn list_events(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let result = GlobalEvents {
        tempo_events: app
            .tempo_events
            .iter()
            .map(|e| TempoEventView {
                bar: wire_bar(e.bar),
                bpm: e.bpm,
            })
            .collect(),
        signature_events: app
            .signature_events
            .iter()
            .map(|e| SignatureEventView {
                bar: wire_bar(e.bar),
                numerator: e.numerator,
                denominator: e.denominator,
            })
            .collect(),
        revision: app.revision(),
    };
    (super::success(request, &result), Task::none())
}

// ---------------------------------------------------------------------------
// Shared helpers for the whole `global.*` namespace
// ---------------------------------------------------------------------------

/// App 0-based bar -> 1-based wire bar.
fn wire_bar(bar: u32) -> u32 {
    bar + 1
}

/// 1-based wire bar -> app 0-based bar, rejecting bar 0 rather than
/// underflowing to the last bar of the song.
///
/// Not called yet — `list_events` only converts the other way. It is
/// established here rather than inlined later so that every `global.*`
/// mutator (slices #1382-#1384) reads a bar the same way.
#[allow(dead_code)]
pub(super) fn state_bar(bar: u32) -> Result<u32, RpcError> {
    bar.checked_sub(1)
        .ok_or_else(|| RpcError::invalid_params("bars are 1-based; bar must be at least 1"))
}

/// Index of the tempo event sitting exactly on `bar` (1-based wire bar),
/// or `None` when no tempo change starts there.
///
/// Resolve and use immediately: the vector re-sorts on every mutation.
/// Unused until the mutating slices land; see [`state_bar`].
#[allow(dead_code)]
pub(super) fn tempo_event_index(app: &Resonance, bar: u32) -> Option<usize> {
    let bar = state_bar(bar).ok()?;
    app.tempo_events.iter().position(|e| e.bar == bar)
}

/// Index of the signature event sitting exactly on `bar` (1-based wire
/// bar), or `None` when no meter change starts there.
///
/// Resolve and use immediately: the vector re-sorts on every mutation.
/// Unused until the mutating slices land; see [`state_bar`].
#[allow(dead_code)]
pub(super) fn signature_event_index(app: &Resonance, bar: u32) -> Option<usize> {
    let bar = state_bar(bar).ok()?;
    app.signature_events.iter().position(|e| e.bar == bar)
}
