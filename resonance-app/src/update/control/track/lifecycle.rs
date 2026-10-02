//! Track lifecycle: `track.add` / `rename` / `delete`, and the two
//! plugin adds (`track.add_instrument` / `add_effect`) that give a fresh
//! track something to make sound with.

use super::{ack, find_track, frozen_reject, not_found_track, reject};
use crate::message::{Message, PluginMessage, TrackMessage};
use crate::update::control::{run_via_update, success, view_model};
use crate::update::plugin_replace::{self, ReplaceKind};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::track::{
    self, AddParams, AddPluginParams, AddResult, DeleteParams, RenameParams, SetColorParams,
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
    let track_id = app.allocate_track_id();
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
// track.rename / track.set_color / track.delete
// ---------------------------------------------------------------------------

pub(super) fn rename(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: RenameParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(current) = find_track(app, params.track_id.0).map(|t| t.name.clone()) else {
        return not_found_track(request, params.track_id.0);
    };
    // Trimmed, and a rename to the current name is acknowledged without
    // an edit (no undo step, no dirty) — as `bus.rename`.
    let name = params.name.trim();
    if name.is_empty() {
        return reject(request, RpcError::invalid_params("track name must not be empty"));
    }
    if name == current {
        return (ack(app, request), Task::none());
    }
    let task = run_via_update(
        app,
        Message::Track(TrackMessage::SetTrackName(params.track_id.0, name.to_owned())),
    );
    (ack(app, request), task)
}

pub(super) fn set_color(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetColorParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(target) = find_track(app, params.track_id.0) else {
        return not_found_track(request, params.track_id.0);
    };
    if target.sub_track.is_some() {
        return reject(
            request,
            RpcError::invalid_params("sub-tracks follow their parent's colour"),
        );
    }
    let Some(color) = track::parse_hex_color(&params.color) else {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "color {:?} is not a \"#rrggbb\" hex colour",
                params.color
            )),
        );
    };
    let task = run_via_update(
        app,
        Message::Track(TrackMessage::SetTrackColor(params.track_id.0, color)),
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
    let confirm_task = if app.modals.confirm_delete_track == Some(params.track_id.0) {
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

fn bank_record_use(app: &Resonance, clap_id: &str, preset_id: &str) -> Result<(), String> {
    crate::plugin_preset_library::library(app)
        .record_use(clap_id, preset_id)
        .map(|_| ())
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
    // `AddPluginToTrackWithId` is a frozen-input edit (gates.rs): the
    // gate would swallow the add, and this handler would still fabricate
    // a `{slot, occurrence}` reply for a plugin that never landed.
    if let Some(e) = frozen_reject(app, params.track_id.0) {
        return reject(request, e);
    }
    let Some(plugin) = app
        .plugin_catalog
        .available_plugins
        .iter()
        .find(|p| p.clap_plugin_id == params.plugin_id)
        .cloned()
    else {
        let valid: Vec<&str> = app
            .plugin_catalog
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
    if matches!(role, PluginRole::Effect) && plugin.is_instrument {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "plugin {:?} is an instrument; an effect slot is handed the track's \
                 audio and has no notes to play — add it with track.add_instrument",
                params.plugin_id
            )),
        );
    }
    // A preset to load onto it, checked before anything is added.
    let preset = match params.preset.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => match crate::update::control::plugin_presets::resolve_add_preset(
            app,
            &params.plugin_id,
            p,
        ) {
            Ok(found) => Some(found),
            Err(e) => return reject(request, e),
        },
        None => None,
    };
    // `add_instrument` SETS the track's instrument (CTL-04). A track has
    // one sound source: appending a second instrument doubled the CPU,
    // let the new one overwrite the old one's output, and left every
    // `plugin_id`-less param edit resolving to the FIRST (unheard) one.
    // So an existing instrument is swapped in place — the same path as
    // `track.replace_effect {slot}` — and re-setting the one already
    // loaded is a no-op, which makes a retry genuinely safe.
    if matches!(role, PluginRole::Instrument) {
        let t = find_track(app, params.track_id.0).expect("track checked above");
        let existing = view_model::plugin_entries(app, t)
            .into_iter()
            .find(|e| e.kind == track::PluginKind::Instrument)
            .and_then(|e| {
                t.plugins
                    .get(e.slot as usize)
                    .map(|p| (e.slot, p.instance_id))
            });
        if let Some((slot, instance_id)) = existing {
            let kind = plugin_replace::classify(app, instance_id, &params.plugin_id);
            let task = match kind {
                None | Some(ReplaceKind::AlreadyLoaded) => Task::none(),
                Some(_) => run_via_update(
                    app,
                    Message::Plugin(PluginMessage::ReplacePlugin {
                        instance_id,
                        plugin,
                    }),
                ),
            };
            if let Some(found) = &preset {
                // The instrument now in the slot: the same one (load now,
                // its params are known) or its replacement (load on the
                // echo, like a fresh add).
                let now = find_track(app, params.track_id.0)
                    .and_then(|t| t.plugins.get(slot as usize))
                    .map(|p| p.instance_id);
                match (kind, now) {
                    (Some(ReplaceKind::AlreadyLoaded), Some(id)) => {
                        // Nothing to add: the preset is a plain recall on
                        // the instrument that is there — one recorded
                        // edit, one revision, like `load_plugin_preset`.
                        let message = crate::update::control::plugin_presets::host_load_message(
                            app,
                            id,
                            &params.plugin_id,
                            &found.id,
                            crate::update::control::plugin_presets::wire_source(found.source),
                        );
                        match message {
                            Ok(message) => {
                                let _ = bank_record_use(app, &params.plugin_id, &found.id);
                                let recall = run_via_update(app, message);
                                let result = track::AddPluginResult {
                                    plugin_id: params.plugin_id,
                                    occurrence: 0,
                                    slot,
                                    revision: app.revision(),
                                };
                                return (success(request, &result), Task::batch([task, recall]));
                            }
                            Err(e) => return reject(request, e),
                        }
                    }
                    (Some(_), Some(id)) => {
                        crate::update::control::plugin_presets::park_add_preset(
                            app,
                            id,
                            &params.plugin_id,
                            found,
                        );
                    }
                    _ => {}
                }
            }
            let result = track::AddPluginResult {
                plugin_id: params.plugin_id,
                occurrence: 0,
                slot,
                revision: app.revision(),
            };
            return (success(request, &result), task);
        }
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

    // The GUI's add is asynchronous: it sends `AddPlugin` and waits for
    // the engine to instantiate the plugin and echo `PluginAdded` before
    // mirroring a slot (the id itself is app-allocated too since ARCH-04
    // D-1, but nothing is mirrored until the echo lands). A control
    // client that issued `track.set_plugin_param` immediately afterwards
    // therefore got "... it carries: []", which reads as "the add
    // failed" (ba doc #273, todo #1234). So mirror a placeholder slot
    // right here instead of waiting for the echo — the plugin is
    // addressable in the SAME update cycle as this reply. The GUI path
    // is untouched: it still waits, via `AddPluginToTrack` rather than
    // this `...WithId` variant.
    let instance_id = app.allocate_plugin_id();
    if let Some(found) = &preset {
        crate::update::control::plugin_presets::park_add_preset(
            app,
            instance_id,
            &params.plugin_id,
            found,
        );
    }
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
