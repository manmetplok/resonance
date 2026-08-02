//! `bus.*` control methods (ba doc #273, todo #1228).
//!
//! Busses are the stage between tracks and master: a group summed to one
//! point so it can be compressed, EQ'd and levelled as a unit. All of it
//! already existed (`BusMessage`, `update/bus.rs`, `ProjectState.busses`,
//! `AudioCommand::AddBus`) and was simply unreachable from the control
//! API, so no client could build a drum bus.
//!
//! Listing is deliberately absent: `song.summary` / `song.tracks` already
//! report busses as tracks with `kind: "bus"`, from a distinct id range.
//!
//! `bus.create` allocates the id APP-SIDE and passes it to the engine as
//! `id_hint`, because the plain `AddBus` path lets the engine allocate
//! and echo the id asynchronously — a reply that has to carry `bus_id`
//! cannot wait for that.

use crate::message::{BusMessage, Message};
use crate::state::BusState;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::bus::{
    self, CreateParams, CreateResult, DeleteParams, SetVolumeParams,
};
use resonance_control::methods::mixer::{VOLUME_DB_MAX, VOLUME_DB_MIN};
use resonance_control::{MutationAck, Request, Response, RpcError};

/// Handle a `bus.*` request, or `None` when `method` belongs to another
/// namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let out = match request.method.as_str() {
        bus::CREATE => create(app, request),
        bus::DELETE => delete(app, request),
        bus::SET_VOLUME => set_volume(app, request),
        _ => return None,
    };
    Some(out)
}

fn reject(request: &Request, error: RpcError) -> (Response, Task<Message>) {
    (super::failure(request, error), Task::none())
}

fn ack(app: &Resonance, request: &Request) -> Response {
    super::success(request, &MutationAck { revision: app.revision() })
}

fn find_bus(app: &Resonance, id: u64) -> Option<&BusState> {
    app.registry.busses.iter().find(|b| b.id == id)
}

fn not_found_bus(request: &Request, id: u64) -> (Response, Task<Message>) {
    reject(request, RpcError::not_found(format!("no bus with id {id}")))
}

fn create(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: CreateParams = match super::optional_params(request) {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let name = match params.name {
        Some(name) if name.trim().is_empty() => {
            return reject(request, RpcError::invalid_params("bus name must not be empty"))
        }
        Some(name) => name,
        None => format!("Bus {}", app.registry.busses.len() + 1),
    };

    // App-side id so the reply can return it immediately; the engine
    // bumps its own allocator past any hint it receives.
    let bus_id = app.registry.allocate_return_bus_id();
    let task = super::run_via_update(
        app,
        Message::Bus(BusMessage::AddBusWithId { id: bus_id, name }),
    );
    let result = CreateResult {
        bus_id: resonance_control::ids::TrackId(bus_id),
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

fn delete(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: DeleteParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_bus(app, params.bus_id.0).is_none() {
        return not_found_bus(request, params.bus_id.0);
    }
    if !params.confirm {
        // `update/bus.rs` re-routes members to master rather than
        // silencing them, so say exactly that instead of implying loss.
        let members: Vec<&str> = app
            .registry
            .tracks
            .iter()
            .filter(|t| t.output == resonance_audio::types::TrackOutput::Bus(params.bus_id.0))
            .map(|t| t.name.as_str())
            .collect();
        return reject(
            request,
            RpcError::needs_confirmation(format!(
                "deleting bus {} re-routes {} track(s) to master ({}) and discards the bus's \
                 own level and effects; re-send with \"confirm\": true",
                params.bus_id,
                members.len(),
                if members.is_empty() {
                    "none".to_owned()
                } else {
                    members.join(", ")
                }
            )),
        );
    }
    let task = super::run_via_update(app, Message::Bus(BusMessage::RemoveBus(params.bus_id.0)));
    (ack(app, request), task)
}

fn set_volume(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetVolumeParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_bus(app, params.bus_id.0).is_none() {
        return not_found_bus(request, params.bus_id.0);
    }
    if !params.volume_db.is_finite() {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "volume_db must be finite (got {}); {VOLUME_DB_MIN} dB is the app's silence \
                 floor, not -inf",
                params.volume_db
            )),
        );
    }
    if !(VOLUME_DB_MIN..=VOLUME_DB_MAX).contains(&params.volume_db) {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "volume_db must be within {VOLUME_DB_MIN}..={VOLUME_DB_MAX} dB — the range the \
                 mixer fader spans — got {}",
                params.volume_db
            )),
        );
    }
    let task = super::run_via_update(
        app,
        Message::Bus(BusMessage::SetBusVolume(params.bus_id.0, params.volume_db)),
    );
    (ack(app, request), task)
}
