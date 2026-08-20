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
//! Routing is necessary but not sufficient, and the two kinds are NOT
//! symmetric about it. `UpdateSignatureEvent` hits the `Record` arm, so
//! one dispatch is one undo entry. `UpdateTempoEvent` does not: it is
//! the GUI's drag-move message and is classified `Skip`, its entry
//! coming from the `StartTempoDrag` (`Begin`) / `EndTempoDrag`
//! (`Commit`) bracket around it. [`edit_tempo_event`] therefore
//! dispatches all three, and that is the single most load-bearing line
//! in this file — bare `UpdateTempoEvent` applies, reads back, and acks
//! correctly while being permanently un-undoable.
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
    self as proto, AddSignatureEventParams, AddTempoEventParams, EditSignatureEventParams,
    EditTempoEventParams, GlobalEvents, SignatureEventView, TempoEventView,
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
        proto::EDIT_TEMPO_EVENT => edit_tempo_event(app, request),
        proto::EDIT_SIGNATURE_EVENT => edit_signature_event(app, request),
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

/// `global.edit_tempo_event` — retune, or relocate, the tempo event that
/// already sits on a bar.
///
/// # The undo bracket, which is the whole reason this is not a one-liner
///
/// `UpdateTempoEvent` is the GUI's drag-MOVE message, so `undo::classify`
/// maps it to `UndoAction::Skip` — a drag would otherwise push one undo
/// entry per mouse-move. Its entry comes from the bracket AROUND it:
/// `StartTempoDrag` is `UndoAction::Begin` (it takes the pre-gesture
/// snapshot) and `EndTempoDrag` is `UndoAction::Commit` (it pushes the
/// entry and bumps the revision).
///
/// Dispatching `UpdateTempoEvent` on its own would therefore produce an
/// edit that applies, reads back correctly, and acks with a revision —
/// and is permanently un-undoable. Nothing about the handler would look
/// wrong, and no test that does not exercise undo could see it. So all
/// three messages go, in order, mirroring the way `transport::set_tempo`
/// routes `SetBpmText` + `CommitBpm` (ba doc #286 §3).
///
/// `EndTempoDrag` earns its place twice over: it also re-sorts the list
/// and runs `rebuild_and_send_tempo`, which is what turns `new_bar` into
/// a sorted move that reaches the audio engine.
fn edit_tempo_event(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: EditTempoEventParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    // Check the address is a bar at all before reporting it as empty, so
    // bar 0 reads as "bars are 1-based" rather than as "no event there".
    if let Err(e) = state_bar(params.bar) {
        return reject(request, e);
    }
    // Edit addresses an event; it never creates one. See `no_event_at`.
    let index = match tempo_event_index(app, params.bar) {
        Some(index) => index,
        None => return reject(request, no_event_at("tempo", params.bar, proto::ADD_TEMPO_EVENT)),
    };
    if params.bpm.is_none() && params.new_bar.is_none() {
        return reject(request, nothing_to_change("bpm and/or new_bar"));
    }
    // Refuse rather than clamp, exactly as the add path does. The domain
    // message clamps to the same range for the GUI's drag, so without
    // this check a wire request for 500 BPM would silently become 300
    // and the two ways into this list would disagree about what a legal
    // tempo is (ba doc #286 §2).
    if let Some(bpm) = params.bpm {
        if let Err(e) = super::validate_bpm(bpm as f64) {
            return reject(request, e);
        }
    }
    let target_bar = match params.new_bar {
        None => app.tempo_events[index].bar,
        Some(new_bar) => {
            // `UpdateTempoEvent` pins `event.bar = 0` for index 0, so
            // moving the initial event is a SILENT no-op in the GUI's
            // own path. Over the wire that is the failure mode this
            // namespace exists to remove: the client is told `ok`, reads
            // the list, and finds the event where it was.
            if index == 0 {
                return reject(
                    request,
                    RpcError::invalid_params(format!(
                        "bar 1 carries the song's initial tempo and cannot be moved to bar \
                         {new_bar} — it is what the start of the song means. Its bpm can be \
                         changed here; a tempo change elsewhere is a separate event, added \
                         with {}",
                        proto::ADD_TEMPO_EVENT
                    )),
                );
            }
            let state = match state_bar(new_bar) {
                Ok(bar) => bar,
                Err(e) => return reject(request, e),
            };
            // One bar, one tempo — the rule `AddTempoEvent`'s upsert
            // enforces (ba todo #1382). A move onto an occupied bar
            // would slip a duplicate past it through the back door,
            // leaving a bar that `tempo_event_index` can no longer
            // address unambiguously.
            if new_bar != params.bar && tempo_event_index(app, new_bar).is_some() {
                return reject(
                    request,
                    RpcError::invalid_params(format!(
                        "bar {new_bar} already carries a tempo event and a bar holds at most \
                         one; edit that event instead (bar: {new_bar}), or remove it first"
                    )),
                );
            }
            state
        }
    };
    let bpm = params.bpm.unwrap_or(app.tempo_events[index].bpm);

    let start = super::run_via_update(
        app,
        Message::GlobalTrack(GlobalTrackMessage::StartTempoDrag(index)),
    );
    let edit = super::run_via_update(
        app,
        Message::GlobalTrack(GlobalTrackMessage::UpdateTempoEvent {
            index,
            bar: target_bar,
            bpm,
        }),
    );
    let end = super::run_via_update(app, Message::GlobalTrack(GlobalTrackMessage::EndTempoDrag));
    ack_task(app, request, Task::batch([start, edit, end]))
}

/// `global.edit_signature_event` — change the meter of the event that
/// already sits on a bar.
///
/// No bracket here, and that asymmetry with [`edit_tempo_event`] is real
/// rather than an oversight: `UpdateSignatureEvent` has no drag gesture
/// behind it, so it falls to `undo::classify`'s
/// `Message::GlobalTrack(_) => UndoAction::Record` arm and one dispatch
/// is one complete undo entry.
///
/// It also has no `new_bar`: the domain message addresses an event by
/// index and rewrites only its numerator/denominator, so a meter event's
/// bar is not editable in place. Moving one is an add at the new bar
/// plus a removal of the old, which keeps the list's one-meter-per-bar
/// invariant in the upsert that already enforces it.
fn edit_signature_event(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: EditSignatureEventParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    // As in `edit_tempo_event`: bar 0 is not an empty bar, it is not a
    // bar.
    if let Err(e) = state_bar(params.bar) {
        return reject(request, e);
    }
    let index = match signature_event_index(app, params.bar) {
        Some(index) => index,
        None => {
            return reject(
                request,
                no_event_at("meter", params.bar, proto::ADD_SIGNATURE_EVENT),
            )
        }
    };
    if params.numerator.is_none() && params.denominator.is_none() {
        return reject(request, nothing_to_change("numerator and/or denominator"));
    }
    // Omitted fields keep their current value, so the pair is validated
    // as the meter it will actually become — 7/8 edited to `numerator: 5`
    // is validated as 5/8, not as a lone 5.
    let numerator = params.numerator.unwrap_or(app.signature_events[index].numerator);
    let denominator = params
        .denominator
        .unwrap_or(app.signature_events[index].denominator);
    if let Err(e) = super::validate_time_signature(numerator, denominator) {
        return reject(request, e);
    }

    let task = super::run_via_update(
        app,
        Message::GlobalTrack(GlobalTrackMessage::UpdateSignatureEvent {
            index,
            numerator,
            denominator,
        }),
    );
    ack_task(app, request, task)
}

// ---------------------------------------------------------------------------
// Shared helpers for the whole `global.*` namespace
// ---------------------------------------------------------------------------

/// "No tempo/meter event at bar N" — the miss every by-bar `edit_*` (and,
/// from ba todo #1384, `remove_*`) reports.
///
/// `edit_*` deliberately refuses an empty bar instead of upserting like
/// `add_*` does. A client that mistakes which bar carries the change it
/// means to adjust is wrong about the song, and inventing the event would
/// hide that behind a plausible-looking `ok` — while leaving the event it
/// actually meant to edit untouched. The error names the add method so
/// the recovery is one call away when creating really was the intent.
fn no_event_at(kind: &str, bar: u32, add_method: &str) -> RpcError {
    RpcError::invalid_params(format!(
        "no {kind} event at bar {bar} — edit changes an event that is already there. \
         Read the track with {} to see which bars carry events, or create one with {add_method}",
        proto::LIST_EVENTS
    ))
}

/// "You asked for an edit and named no field" — refused rather than
/// acknowledged as a no-op, so a caller that omitted the field it meant
/// to send learns that instead of reading an unchanged track back and
/// having to guess whether the call or the song is wrong. It also keeps
/// an empty edit out of the undo history.
fn nothing_to_change(fields: &str) -> RpcError {
    RpcError::invalid_params(format!(
        "nothing to change: pass {fields}. Omitted fields keep their current value, \
         so a call that omits all of them would be a no-op"
    ))
}

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
/// Resolve and use immediately: the vector re-sorts on every mutation —
/// [`edit_tempo_event`] resolves, validates and dispatches without ever
/// letting an index outlive the call, and `remove_tempo_event` (ba todo
/// #1384) will do the same.
///
/// `None` is what makes `edit_*` refuse an empty bar rather than upsert
/// like `add_*` does; the adds need no lookup at all, because
/// `AddTempoEvent` upserts by bar itself.
pub(super) fn tempo_event_index(app: &Resonance, bar: u32) -> Option<usize> {
    let bar = state_bar(bar).ok()?;
    app.tempo_events.iter().position(|e| e.bar == bar)
}

/// Index of the signature event sitting exactly on `bar` (1-based wire
/// bar), or `None` when no meter change starts there.
///
/// Resolve and use immediately, for the same reason as
/// [`tempo_event_index`]. [`edit_signature_event`] is the caller today;
/// `remove_signature_event` (ba todo #1384) joins it.
pub(super) fn signature_event_index(app: &Resonance, bar: u32) -> Option<usize> {
    let bar = state_bar(bar).ok()?;
    app.signature_events.iter().position(|e| e.bar == bar)
}
