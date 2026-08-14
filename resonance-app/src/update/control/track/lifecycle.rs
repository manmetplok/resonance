//! Track lifecycle: `track.add` / `rename` / `delete`, and the two
//! plugin adds (`track.add_instrument` / `add_effect`) that give a fresh
//! track something to make sound with.

use super::{ack, find_track, not_found_track, reject};
use crate::message::{Message, PluginMessage, TrackMessage};
use crate::update::control::{run_via_update, success};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::track::{
    self, AddParams, AddPluginParams, AddResult, DeleteParams, RenameParams,
};
use resonance_control::{Request, Response, RpcError, TrackKind};

// ---------------------------------------------------------------------------
// track.add
// ---------------------------------------------------------------------------

pub(super) fn add(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
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
        TrackKind::External => ControlTrackKind::External,
        TrackKind::Bus | TrackKind::Unknown => {
            return reject(
                request,
                RpcError::invalid_params(format!(
                    "cannot add a track of kind {:?}; use instrument|drums|vocal|audio|external",
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
    let task = run_via_update(
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
    (success(request, &result), task)
}

// ---------------------------------------------------------------------------
// track.rename / track.delete
// ---------------------------------------------------------------------------

pub(super) fn rename(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
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
    let task = run_via_update(
        app,
        Message::Track(TrackMessage::SetTrackName(params.track_id.0, params.name)),
    );
    (ack(app, request), task)
}

pub(super) fn delete(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
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
    let request_task = run_via_update(
        app,
        Message::Track(TrackMessage::RequestRemoveTrack(params.track_id.0)),
    );
    let confirm_task = if app.confirm_delete_track == Some(params.track_id.0) {
        run_via_update(app, Message::Track(TrackMessage::ConfirmRemoveTrack))
    } else {
        Task::none()
    };
    (ack(app, request), Task::batch([request_task, confirm_task]))
}

// ---------------------------------------------------------------------------
// track.add_instrument / track.add_effect
// ---------------------------------------------------------------------------

pub(super) fn add_instrument(
    app: &mut Resonance,
    request: &Request,
) -> (Response, Task<Message>) {
    add_plugin(app, request, PluginRole::Instrument)
}

pub(super) fn add_effect(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
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
            .filter(|p| p.is_instrument == matches!(role, PluginRole::Instrument))
            .map(|p| p.clap_plugin_id.as_str())
            .collect();
        // An empty list here is nearly always an unbuilt checkout rather
        // than a wrong id: the first-party plugins are CLAP bundles the
        // scanner only sees once they are built. Without this the caller
        // sees "valid ids: []" and reasonably concludes the app ships no
        // instruments at all (ba doc #270 §1).
        let hint = if valid.is_empty() {
            " — the catalog holds none of this kind, which usually means the \
             first-party plugins have not been built; run scripts/bundle.sh \
             and rescan"
        } else {
            ""
        };
        return reject(
            request,
            RpcError::not_found(format!(
                "unknown plugin id {:?}; valid ids: {:?}{}",
                params.plugin_id, valid, hint
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
    // Compute the handle BEFORE dispatching: the engine appends
    // (`push_plugin`), so the new plugin lands at the current chain
    // length, and its occurrence is the number of copies already there.
    let (slot, occurrence) = {
        // `find_track` succeeded above.
        let t = find_track(app, params.track_id.0).expect("track checked above");
        (
            t.plugins.len() as u32,
            t.plugins
                .iter()
                .filter(|p| p.clap_plugin_id == params.plugin_id)
                .count() as u32,
        )
    };

    // The GUI's add is asynchronous: the engine allocates the instance
    // id, instantiates the plugin and echoes `PluginAdded`, and only
    // then does the app mirror a slot. A control client that issued
    // `track.set_plugin_param` immediately afterwards therefore got
    // "... it carries: []", which reads as "the add failed" (ba doc
    // #273, todo #1234). So allocate the id app-side, pass it as
    // `id_hint`, and let the message handler mirror a placeholder slot —
    // the plugin is addressable in the SAME update cycle as this reply.
    // The GUI path is untouched and stays engine-allocated.
    let instance_id = app.allocate_control_plugin_id();
    let task = run_via_update(
        app,
        Message::Plugin(PluginMessage::AddPluginToTrackWithId {
            track_id: params.track_id.0,
            instance_id,
            plugin,
        }),
    );
    let result = track::AddPluginResult {
        plugin_id: params.plugin_id,
        occurrence,
        slot,
        revision: app.revision(),
    };
    (success(request, &result), task)
}
