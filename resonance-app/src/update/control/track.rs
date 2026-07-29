//! `track.*` and `mixer.*` control methods (ba doc #265, todo #1152).
//!
//! Every mutation synthesizes the existing `TrackMessage` /
//! `PluginMessage` and routes it through the FULL `update()` path via
//! [`super::run_via_update`], so remote edits are undoable exactly like
//! the GUI (Cmd-Z reverts them). Params are validated before dispatch so
//! rejections are precise (`invalid_params` / `not_found` /
//! `needs_confirmation`). Mutating replies carry the revision counter;
//! `track.add`/`track.rename`/plugin ops also echo the updated
//! [`TrackSummary`] via [`AddResult`]/[`super::song`] helpers.
//!
//! Track creation is asynchronous engine-side (the registry mirrors the
//! track on the `TrackAdded` echo), so `track.add` allocates the id
//! app-side and passes it as `id_hint` — the same pattern the
//! external-instrument add uses — so the reply can return the real
//! `track_id` immediately.

use crate::message::{Message, PluginMessage, TrackMessage};
use crate::state::TrackState;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::mixer::{
    self, SetMuteParams, SetPanParams, SetSoloParams, SetVolumeParams,
};
use resonance_control::methods::track::{
    self, AddParams, AddPluginParams, AddResult, DeleteParams, RenameParams,
};
use resonance_control::{MutationAck, Request, Response, RpcError, TrackKind};

/// Handle a `track.*` / `mixer.*` request, or `None` when `method`
/// belongs to another namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let out = match request.method.as_str() {
        track::ADD => add(app, request),
        track::RENAME => rename(app, request),
        track::DELETE => delete(app, request),
        track::ADD_INSTRUMENT => add_instrument(app, request),
        track::ADD_EFFECT => add_effect(app, request),
        mixer::SET_VOLUME => set_volume(app, request),
        mixer::SET_PAN => set_pan(app, request),
        mixer::SET_MUTE => set_mute(app, request),
        mixer::SET_SOLO => set_solo(app, request),
        _ => return None,
    };
    Some(out)
}

fn reject(request: &Request, error: RpcError) -> (Response, Task<Message>) {
    (super::failure(request, error), Task::none())
}

/// The `MutationAck` reply carrying the post-edit revision.
fn ack(app: &Resonance, request: &Request) -> Response {
    super::success(request, &MutationAck { revision: app.revision() })
}

/// Look up a live track by wire id, or `None`.
fn find_track(app: &Resonance, id: u64) -> Option<&TrackState> {
    app.registry.tracks.iter().find(|t| t.id == id)
}

fn not_found_track(request: &Request, id: u64) -> (Response, Task<Message>) {
    reject(request, RpcError::not_found(format!("no track with id {id}")))
}

// ---------------------------------------------------------------------------
// track.add
// ---------------------------------------------------------------------------

fn add(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: AddParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };

    use crate::state::ControlTrackKind;
    let kind = match params.kind {
        TrackKind::Instrument => ControlTrackKind::Instrument,
        TrackKind::Drums => ControlTrackKind::Drums,
        TrackKind::Vocal => ControlTrackKind::Vocal,
        TrackKind::Audio => ControlTrackKind::Audio,
        TrackKind::Bus | TrackKind::Unknown => {
            return reject(
                request,
                RpcError::invalid_params(format!(
                    "cannot add a track of kind {:?}; use instrument|drums|vocal|audio",
                    params.kind
                )),
            )
        }
    };

    // Allocate the id app-side so the reply returns it immediately (the
    // engine echoes `*TrackAdded { id }`, mirroring the track into the
    // registry a beat later — same pattern as the external-instrument
    // add). `AddControlTrack` carries the hint + name + drums promotion
    // as one undoable step.
    let track_id = app.registry.allocate_sub_track_id();
    let task = super::run_via_update(
        app,
        Message::Track(TrackMessage::AddControlTrack {
            id: track_id,
            kind,
            name: params.name.clone(),
        }),
    );

    let result = AddResult {
        track_id: resonance_control::ids::TrackId(track_id),
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

// ---------------------------------------------------------------------------
// track.rename / track.delete
// ---------------------------------------------------------------------------

fn rename(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: RenameParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_track(app, params.track_id.0).is_none() {
        return not_found_track(request, params.track_id.0);
    }
    if params.name.trim().is_empty() {
        return reject(request, RpcError::invalid_params("track name must not be empty"));
    }
    let task = super::run_via_update(
        app,
        Message::Track(TrackMessage::SetTrackName(params.track_id.0, params.name)),
    );
    (ack(app, request), task)
}

fn delete(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: DeleteParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_track(app, params.track_id.0).is_none() {
        return not_found_track(request, params.track_id.0);
    }
    if !params.confirm {
        let audio = app.clips.iter().filter(|c| c.track_id == params.track_id.0).count();
        let midi = app
            .midi_clips
            .iter()
            .filter(|c| c.track_id == params.track_id.0)
            .count();
        return reject(
            request,
            RpcError::needs_confirmation(format!(
                "deleting track {} removes {audio} audio clip(s) and {midi} MIDI clip(s); \
                 re-send with \"confirm\": true",
                params.track_id
            )),
        );
    }
    // ConfirmRemoveTrack consumes `confirm_delete_track`; the request
    // may target a track with no content (which never sets that field),
    // so route RequestRemoveTrack (deletes immediately when empty) and,
    // for a track with content, the confirm it just staged.
    let request_task = super::run_via_update(
        app,
        Message::Track(TrackMessage::RequestRemoveTrack(params.track_id.0)),
    );
    let confirm_task = if app.confirm_delete_track == Some(params.track_id.0) {
        super::run_via_update(app, Message::Track(TrackMessage::ConfirmRemoveTrack))
    } else {
        Task::none()
    };
    (ack(app, request), Task::batch([request_task, confirm_task]))
}

// ---------------------------------------------------------------------------
// track.add_instrument / track.add_effect
// ---------------------------------------------------------------------------

fn add_instrument(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    add_plugin(app, request, PluginRole::Instrument)
}

fn add_effect(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    add_plugin(app, request, PluginRole::Effect)
}

enum PluginRole {
    Instrument,
    Effect,
}

fn add_plugin(
    app: &mut Resonance,
    request: &Request,
    role: PluginRole,
) -> (Response, Task<Message>) {
    let params: AddPluginParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_track(app, params.track_id.0).is_none() {
        return not_found_track(request, params.track_id.0);
    }
    let Some(plugin) = app
        .available_plugins
        .iter()
        .find(|p| p.clap_plugin_id == params.plugin_id)
        .cloned()
    else {
        let valid: Vec<&str> = app
            .available_plugins
            .iter()
            .map(|p| p.clap_plugin_id.as_str())
            .collect();
        return reject(
            request,
            RpcError::not_found(format!(
                "unknown plugin id {:?}; valid ids: {:?}",
                params.plugin_id, valid
            )),
        );
    };
    if matches!(role, PluginRole::Instrument) && !plugin.is_instrument {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "plugin {:?} is not an instrument; use track.add_effect",
                params.plugin_id
            )),
        );
    }
    // AddPluginToTrack appends to the chain. On an instrument track the
    // instrument sits at slot 0 (the engine's add path handles the
    // instrument-vs-effect placement); the control surface exposes the
    // same append the GUI uses.
    let task = super::run_via_update(
        app,
        Message::Plugin(PluginMessage::AddPluginToTrack(params.track_id.0, plugin)),
    );
    (ack(app, request), task)
}

// ---------------------------------------------------------------------------
// mixer.*
// ---------------------------------------------------------------------------

fn set_volume(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetVolumeParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_track(app, params.track_id.0).is_none() {
        return not_found_track(request, params.track_id.0);
    }
    if !params.volume.is_finite() || params.volume < 0.0 {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "volume must be a non-negative linear gain (got {})",
                params.volume
            )),
        );
    }
    // Wire is linear gain; the app stores/sends dB. Silence maps to a
    // deep floor rather than -inf so the fader state stays finite.
    let db = if params.volume <= 0.0 {
        -80.0
    } else {
        20.0 * params.volume.log10()
    };
    let task = super::run_via_update(
        app,
        Message::Track(TrackMessage::SetTrackVolume(params.track_id.0, db)),
    );
    (ack(app, request), task)
}

fn set_pan(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetPanParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_track(app, params.track_id.0).is_none() {
        return not_found_track(request, params.track_id.0);
    }
    if !params.pan.is_finite() || !(-1.0..=1.0).contains(&params.pan) {
        return reject(
            request,
            RpcError::invalid_params(format!("pan must be in -1.0..=1.0 (got {})", params.pan)),
        );
    }
    let task = super::run_via_update(
        app,
        Message::Track(TrackMessage::SetTrackPan(params.track_id.0, params.pan)),
    );
    (ack(app, request), task)
}

fn set_mute(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetMuteParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(track) = find_track(app, params.track_id.0) else {
        return not_found_track(request, params.track_id.0);
    };
    // The GUI has only a toggle; only dispatch when the state actually
    // needs to flip, so an idempotent set never records a no-op undo
    // entry or double-toggles.
    if track.muted == params.muted {
        return (ack(app, request), Task::none());
    }
    let task = super::run_via_update(
        app,
        Message::Track(TrackMessage::ToggleMute(params.track_id.0)),
    );
    (ack(app, request), task)
}

fn set_solo(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetSoloParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(track) = find_track(app, params.track_id.0) else {
        return not_found_track(request, params.track_id.0);
    };
    if track.soloed == params.soloed {
        return (ack(app, request), Task::none());
    }
    let task = super::run_via_update(
        app,
        Message::Track(TrackMessage::ToggleSolo(params.track_id.0)),
    );
    (ack(app, request), task)
}
