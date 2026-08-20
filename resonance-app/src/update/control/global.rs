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
//! # One event per bar: the adds upsert
//!
//! `global.add_tempo_event` / `global.add_signature_event` REPLACE an
//! existing event of their kind at that bar rather than stacking a
//! second one on it. `AddSignatureEvent` always behaved that way;
//! `AddTempoEvent` used to push unconditionally and re-sort, leaving two
//! events on one bar — a duplicate bar in `list_events` and in
//! `song.summary`, and a bar that `tempo_event_index` (a find-first)
//! could no longer address unambiguously.
//!
//! That asymmetry is fixed in the domain message itself (ba todo #1382,
//! `update::global_track`), not papered over here by picking between an
//! add and an edit: the handler would then have to dispatch
//! `UpdateTempoEvent`, which is classified `UndoAction::Skip` because it
//! is the drag-move message, so the "correction" case would silently be
//! the one edit a user could not undo. Fixing the message keeps one code
//! path, keeps the classification at `Record`, and stops the GUI's own
//! double-click from producing the duplicate too.
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

use crate::message::{GlobalTrackMessage, Message};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::global::{
    self as proto, AddSignatureEventParams, AddTempoEventParams, GlobalEvents, SignatureEventView,
    TempoEventView,
};
use resonance_control::{Request, Response, RpcError};

use super::reply::{ack_task, reject};

/// Handle a `global.*` request, or `None` when `method` belongs to
/// another namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let handled = match request.method.as_str() {
        proto::LIST_EVENTS => list_events(app, request),
        proto::ADD_TEMPO_EVENT => add_tempo_event(app, request),
        proto::ADD_SIGNATURE_EVENT => add_signature_event(app, request),
        _ => return None,
    };
    Some(handled)
}

/// `global.list_events` — both global tracks, in bar order.
fn list_events(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let result = GlobalEvents {
        tempo_events: tempo_event_views(app),
        signature_events: signature_event_views(app),
        revision: app.revision(),
    };
    (super::success(request, &result), Task::none())
}

/// `global.add_tempo_event` — a tempo change at a bar, upserting.
///
/// Dispatches the GUI's own `AddTempoEvent`, which since ba todo #1382
/// replaces an existing event at that bar instead of pushing a second
/// one (see [`crate::update::global_track`]): one bar, one tempo, so a
/// repeated call is a correction rather than a pile-up and the bar stays
/// a usable address for `global.edit_*` / `global.remove_*`.
fn add_tempo_event(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: AddTempoEventParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let bar = match state_bar(params.bar) {
        Ok(bar) => bar,
        Err(e) => return reject(request, e),
    };
    if let Err(e) = super::validate_bpm(params.bpm as f64) {
        return reject(request, e);
    }
    let task = super::run_via_update(
        app,
        Message::GlobalTrack(GlobalTrackMessage::AddTempoEvent {
            bar,
            bpm: params.bpm,
        }),
    );
    ack_task(app, request, task)
}

/// `global.add_signature_event` — a meter change at a bar, upserting.
///
/// `AddSignatureEvent` has always overwritten an existing event at the
/// same bar; the meter is validated first so an illegal one is refused
/// with a reason instead of being written to the track.
fn add_signature_event(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: AddSignatureEventParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let bar = match state_bar(params.bar) {
        Ok(bar) => bar,
        Err(e) => return reject(request, e),
    };
    // Shared with `transport.set_time_signature`, which writes the bar-1
    // event of this same track (ba doc #286 §2).
    if let Err(e) = super::validate_time_signature(params.numerator, params.denominator) {
        return reject(request, e);
    }
    let task = super::run_via_update(
        app,
        Message::GlobalTrack(GlobalTrackMessage::AddSignatureEvent {
            bar,
            numerator: params.numerator,
            denominator: params.denominator,
        }),
    );
    ack_task(app, request, task)
}

// ---------------------------------------------------------------------------
// Shared helpers for the whole `global.*` namespace
// ---------------------------------------------------------------------------

/// The tempo track as wire views, in bar order.
///
/// `song.summary` carries the same list (ba todo #1381) so that a client
/// can tell from one read whether the song changes tempo at all. It is
/// built here rather than there so the two answers cannot drift apart —
/// in particular so they cannot disagree about the 0-based state bar to
/// 1-based wire bar conversion.
pub(super) fn tempo_event_views(app: &Resonance) -> Vec<TempoEventView> {
    app.tempo_events
        .iter()
        .map(|e| TempoEventView {
            bar: wire_bar(e.bar),
            bpm: e.bpm,
        })
        .collect()
}

/// The signature track as wire views, in bar order. Shared with
/// `song.summary`; see [`tempo_event_views`].
pub(super) fn signature_event_views(app: &Resonance) -> Vec<SignatureEventView> {
    app.signature_events
        .iter()
        .map(|e| SignatureEventView {
            bar: wire_bar(e.bar),
            numerator: e.numerator,
            denominator: e.denominator,
        })
        .collect()
}

/// App 0-based bar -> 1-based wire bar.
fn wire_bar(bar: u32) -> u32 {
    bar + 1
}

/// 1-based wire bar -> app 0-based bar, rejecting bar 0 rather than
/// underflowing to the last bar of the song.
///
/// Every `global.*` mutator reads its `bar` param through here, so the
/// conversion and the bar-0 refusal are written once for the namespace.
pub(super) fn state_bar(bar: u32) -> Result<u32, RpcError> {
    bar.checked_sub(1)
        .ok_or_else(|| RpcError::invalid_params("bars are 1-based; bar must be at least 1"))
}

/// Index of the tempo event sitting exactly on `bar` (1-based wire bar),
/// or `None` when no tempo change starts there.
///
/// Resolve and use immediately: the vector re-sorts on every mutation.
///
/// Still unused after the add slice (#1382): `add_tempo_event` needs no
/// lookup, because `AddTempoEvent` itself upserts by bar rather than the
/// handler picking between an add and an edit. The index-addressed
/// messages that need this are `UpdateTempoEvent` (#1383) and
/// `remove_tempo_event` (#1384).
#[allow(dead_code)]
pub(super) fn tempo_event_index(app: &Resonance, bar: u32) -> Option<usize> {
    let bar = state_bar(bar).ok()?;
    app.tempo_events.iter().position(|e| e.bar == bar)
}

/// Index of the signature event sitting exactly on `bar` (1-based wire
/// bar), or `None` when no meter change starts there.
///
/// Resolve and use immediately: the vector re-sorts on every mutation.
/// Unused for the same reason as [`tempo_event_index`] — the add path
/// upserts without one; `UpdateSignatureEvent` (#1383) and
/// `remove_signature_event` (#1384) are the callers.
#[allow(dead_code)]
pub(super) fn signature_event_index(app: &Resonance, bar: u32) -> Option<usize> {
    let bar = state_bar(bar).ok()?;
    app.signature_events.iter().position(|e| e.bar == bar)
}
