//! `track.add_send` / `set_send` / `remove_send` (ba doc #273, todo
//! #1229) — the aux-send graph.
//!
//! The engine is the single writer of the send graph: these handlers
//! synthesize the existing `MixerMessage` values and let `AuxSendChanged`
//! / `AuxSendRemoved` update `AuxSendState`. Cyclic routes are validated
//! with the engine's own `aux_send_would_cycle` predicate (not a second
//! implementation) so a route the engine would refuse is reported as
//! `invalid_params` here instead of silently vanishing.
//!
//! NOTE: aux sends are NOT persisted — nothing in
//! `update/project_io/serialize.rs` writes the send graph. That is open
//! ba todo #482; until it lands a send created here is lost on
//! save + reload, and the MCP tool descriptions say so.

use super::{ack, find_track, not_found_track, reject};
use crate::message::{Message, MixerMessage};
use crate::update::control::{run_via_update, success};
use crate::Resonance;
use iced::Task;
use resonance_audio::types::{aux_send_would_cycle, SendSource};
use resonance_control::methods::track;
use resonance_control::{Request, Response, RpcError};

/// Look up a mirrored send by id.
fn find_send(app: &Resonance, send_id: u64) -> Option<resonance_audio::types::AuxSend> {
    app.aux.sends.iter().find(|s| s.id == send_id).copied()
}

fn not_found_send(request: &Request, send_id: u64) -> (Response, Task<Message>) {
    reject(
        request,
        RpcError::not_found(format!(
            "no send with id {send_id}; song.tracks reports each track's sends"
        )),
    )
}

/// Validate a return-bus destination, and the level if one was given.
fn check_send_target(
    app: &Resonance,
    to_bus: u64,
    level_db: Option<f32>,
) -> Result<(), RpcError> {
    if !app.registry.busses.iter().any(|b| b.id == to_bus) {
        let known: Vec<String> = app
            .registry
            .busses
            .iter()
            .map(|b| format!("{} ({})", b.id, b.name))
            .collect();
        return Err(RpcError::not_found(format!(
            "no bus with id {to_bus} to send into; create one with bus.create. Existing \
             busses: [{}]",
            known.join(", ")
        )));
    }
    if let Some(db) = level_db {
        if !db.is_finite()
            || !(track::SEND_LEVEL_DB_MIN..=track::SEND_LEVEL_DB_MAX).contains(&db)
        {
            return Err(RpcError::invalid_params(format!(
                "level_db must be a finite value within {}..={} dB (got {db})",
                track::SEND_LEVEL_DB_MIN,
                track::SEND_LEVEL_DB_MAX
            )));
        }
    }
    Ok(())
}

/// `track.add_send` — tap a track into a return bus.
///
/// The destination is flagged as a return bus as part of the gesture, so
/// a client does not have to know that busses carry a role; that mirrors
/// what `MixerMessage::CreateReturnFromSend` does in the GUI.
pub(super) fn add_send(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: track::AddSendParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_track(app, params.track_id.0).is_none() {
        return not_found_track(request, params.track_id.0);
    }
    if let Err(error) = check_send_target(app, params.to_bus.0, params.level_db) {
        return reject(request, error);
    }

    let source = SendSource::Track(params.track_id.0);
    // The engine's own predicate, not a second one. A track source can
    // never cycle, but running the same check keeps this honest if the
    // source ever widens to busses.
    if aux_send_would_cycle(app.aux.sends.iter(), source, params.to_bus.0, None) {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "a send from track {} into bus {} would create a feedback loop",
                params.track_id, params.to_bus
            )),
        );
    }

    // App-chosen id so the reply carries it; the engine bumps its own
    // allocator past any hint it receives.
    let send_id = app.aux.allocate_control_send_id();
    let role_task = run_via_update(
        app,
        Message::Mixer(MixerMessage::SetBusReturnRole(params.to_bus.0, true)),
    );
    let send_task = run_via_update(
        app,
        Message::Mixer(MixerMessage::AddSendWithId {
            id: send_id,
            source,
            dest: params.to_bus.0,
            level_db: params.level_db.unwrap_or(0.0),
            pre_fader: params.pre_fader.unwrap_or(false),
        }),
    );
    let result = track::AddSendResult {
        send_id: resonance_control::ids::SendId(send_id),
        revision: app.revision(),
    };
    (
        success(request, &result),
        Task::batch([role_task, send_task]),
    )
}

/// `track.set_send` — change level / tap point / enable / destination.
/// Omitted fields keep their current value.
pub(super) fn set_send(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: track::SetSendParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(send) = find_send(app, params.send_id.0) else {
        return not_found_send(request, params.send_id.0);
    };
    if params.level_db.is_none()
        && params.pre_fader.is_none()
        && params.enabled.is_none()
        && params.to_bus.is_none()
    {
        return reject(
            request,
            RpcError::invalid_params(
                "give at least one of level_db, pre_fader, enabled or to_bus",
            ),
        );
    }
    if let Some(to_bus) = params.to_bus {
        if let Err(error) = check_send_target(app, to_bus.0, params.level_db) {
            return reject(request, error);
        }
        if aux_send_would_cycle(
            app.aux.sends.iter(),
            send.source,
            to_bus.0,
            Some(send.id),
        ) {
            return reject(
                request,
                RpcError::invalid_params(format!(
                    "re-routing send {} into bus {to_bus} would create a feedback loop",
                    params.send_id
                )),
            );
        }
    } else if let Some(db) = params.level_db {
        if !db.is_finite()
            || !(track::SEND_LEVEL_DB_MIN..=track::SEND_LEVEL_DB_MAX).contains(&db)
        {
            return reject(
                request,
                RpcError::invalid_params(format!(
                    "level_db must be a finite value within {}..={} dB (got {db})",
                    track::SEND_LEVEL_DB_MIN,
                    track::SEND_LEVEL_DB_MAX
                )),
            );
        }
    }

    // Each existing message edits one field; the engine treats every one
    // as an upsert on the same send id. Toggles are only dispatched when
    // the state actually needs to flip, so an idempotent set is a no-op.
    let mut tasks = Vec::new();
    if let Some(to_bus) = params.to_bus {
        if send.dest != to_bus.0 {
            tasks.push(run_via_update(
                app,
                Message::Mixer(MixerMessage::SetSendDest(send.id, to_bus.0)),
            ));
        }
    }
    if let Some(db) = params.level_db {
        if send.level_db != db {
            tasks.push(run_via_update(
                app,
                Message::Mixer(MixerMessage::SetSendLevel(send.id, db)),
            ));
        }
    }
    if let Some(pre) = params.pre_fader {
        if send.pre_fader != pre {
            tasks.push(run_via_update(
                app,
                Message::Mixer(MixerMessage::ToggleSendPreFader(send.id)),
            ));
        }
    }
    if let Some(enabled) = params.enabled {
        if send.enabled != enabled {
            tasks.push(run_via_update(
                app,
                Message::Mixer(MixerMessage::ToggleSendEnabled(send.id)),
            ));
        }
    }
    (ack(app, request), Task::batch(tasks))
}

/// `track.remove_send` — delete a send outright.
pub(super) fn remove_send(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: track::RemoveSendParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_send(app, params.send_id.0).is_none() {
        return not_found_send(request, params.send_id.0);
    }
    let task = run_via_update(
        app,
        Message::Mixer(MixerMessage::RemoveSend(params.send_id.0)),
    );
    (ack(app, request), task)
}
