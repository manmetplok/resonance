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
    self, SetMuteParams, SetPanParams, SetSoloParams, SetVolumeDbParams, SetVolumeParams,
};
use resonance_control::methods::track::{
    self, AddParams, AddPluginParams, AddResult, DeleteParams, RemoveEffectParams, RenameParams,
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
        track::REMOVE_EFFECT => remove_effect(app, request),
        track::SET_PLUGIN_PARAM => set_plugin_param(app, request),
        mixer::SET_VOLUME => set_volume(app, request),
        mixer::SET_VOLUME_DB => set_volume_db(app, request),
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
// track.remove_effect
// ---------------------------------------------------------------------------

/// `track.remove_effect` — take one effect off a track's insert chain
/// (ba doc #273, todo #1223).
///
/// Until this existed an effect could only ever be APPENDED, and
/// `track.add_effect` adds another instance on every call, so a wrong
/// add was unrecoverable over the control API. Dispatches the existing
/// `PluginMessage::RemovePluginFromTrack` through `run_via_update`, so
/// the removal is undoable like a manual one.
fn remove_effect(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: RemoveEffectParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(t) = find_track(app, params.track_id.0).cloned() else {
        return not_found_track(request, params.track_id.0);
    };
    let entries = super::song::plugin_entries(app, &t);

    let entry = match (params.slot, &params.plugin_id) {
        (Some(_), Some(_)) => {
            return reject(
                request,
                RpcError::invalid_params(
                    "address the effect by slot OR by plugin_id (+ occurrence), not both",
                ),
            )
        }
        (None, None) => {
            return reject(
                request,
                RpcError::invalid_params(format!(
                    "name the effect to remove: slot, or plugin_id (+ occurrence). Track {} \
                     carries [{}]",
                    t.id,
                    chain_description(&entries)
                )),
            )
        }
        (Some(slot), None) => match entries.iter().find(|e| e.slot == slot) {
            Some(entry) => entry,
            None => {
                return reject(
                    request,
                    RpcError::not_found(format!(
                        "track {} has no plugin at slot {slot}; it carries [{}]",
                        t.id,
                        chain_description(&entries)
                    )),
                )
            }
        },
        (None, Some(plugin_id)) => {
            let occurrence = params.occurrence.unwrap_or(0);
            match entries
                .iter()
                .find(|e| &e.plugin_id == plugin_id && e.occurrence == occurrence)
            {
                Some(entry) => entry,
                None => {
                    return reject(
                        request,
                        super::song::unknown_plugin_on_track(app, &t, plugin_id, occurrence),
                    )
                }
            }
        }
    };

    // The instrument is replaced with `track.add_instrument`, never
    // removed here — taking it off would silently mute the track.
    if entry.kind == track::PluginKind::Instrument {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "slot {} on track {} is the track's INSTRUMENT ({}), not an effect; replace it \
                 with track.add_instrument instead of removing it",
                entry.slot, t.id, entry.plugin_id
            )),
        );
    }

    let Some(instance_id) = instance_for(&t, &entry.plugin_id, entry.occurrence) else {
        return reject(
            request,
            RpcError::not_found(format!(
                "plugin {:?} vanished from track {} between lookup and removal",
                entry.plugin_id, t.id
            )),
        );
    };
    let task = super::run_via_update(
        app,
        Message::Plugin(PluginMessage::RemovePluginFromTrack(t.id, instance_id)),
    );
    (ack(app, request), task)
}

/// The chain as `slot:plugin_id` pairs, for error messages that let the
/// caller correct an address rather than guess again.
fn chain_description(entries: &[track::PluginParamsEntry]) -> String {
    entries
        .iter()
        .map(|e| format!("{}:{}", e.slot, e.plugin_id))
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------------------------------------------------------------------------
// track.set_plugin_param
// ---------------------------------------------------------------------------

/// Set one parameter on one plugin (ba doc #272 V-3).
///
/// Routes the same `PluginMessage::SetPluginParam` the GUI's plugin
/// panel sends, so the edit reaches the engine and is undoable.
/// Successive sets of the *same* parameter coalesce into one undo entry
/// (a knob drag is one gesture), but each still bumps `revision` — a
/// remote client polling the counter sees every committed change.
///
/// Addressing mirrors `track.plugin_params`: the CLAP id `song.tracks`
/// reports, plus `occurrence` for a track carrying the same plugin
/// twice.
fn set_plugin_param(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: track::SetPluginParamParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if !params.value.is_finite() {
        return reject(
            request,
            RpcError::invalid_params(format!("value must be finite (got {})", params.value)),
        );
    }
    let Some(t) = find_track(app, params.track_id.0).cloned() else {
        return not_found_track(request, params.track_id.0);
    };

    // Resolve the plugin: an explicit id, else the track's instrument —
    // the common case for "make this synth sound different".
    let entries = super::song::plugin_entries(app, &t);
    let occurrence = params.occurrence.unwrap_or(0);
    let entry = match &params.plugin_id {
        Some(id) => entries
            .iter()
            .find(|e| &e.plugin_id == id && e.occurrence == occurrence),
        None => entries
            .iter()
            .find(|e| e.kind == track::PluginKind::Instrument),
    };
    let Some(entry) = entry else {
        let error = match &params.plugin_id {
            Some(id) => super::song::unknown_plugin_on_track(app, &t, id, occurrence),
            None => RpcError::invalid_params(format!(
                "track {} has no instrument; name a plugin_id (it carries: [{}])",
                t.id,
                entries
                    .iter()
                    .map(|e| e.plugin_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        };
        return reject(request, error);
    };

    // Resolve the parameter by name first, then by numeric CLAP id — a
    // client reading `track.plugin_params` has both, and a name is what
    // it will usually have to hand.
    let wanted = params.param.trim();
    let param = entry
        .params
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(wanted))
        .or_else(|| {
            wanted
                .parse::<u32>()
                .ok()
                .and_then(|id| entry.params.iter().find(|p| p.id == id))
        });
    let Some(param) = param else {
        let known: Vec<&str> = entry.params.iter().map(|p| p.name.as_str()).collect();
        return reject(
            request,
            RpcError::not_found(format!(
                "plugin {:?} has no parameter {wanted:?} (has: [{}])",
                entry.plugin_id,
                known.join(", ")
            )),
        );
    };

    // Reject rather than clamp: silently moving a value the caller asked
    // for is how a mix ends up subtly wrong with nothing to point at.
    if params.value < param.min || params.value > param.max {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "{} must be within {}..={} (got {})",
                param.name, param.min, param.max, params.value
            )),
        );
    }

    // The instance id is the engine's handle; it is not on the wire, so
    // recover it from the same chain position the entry came from.
    let Some(instance_id) = instance_for(&t, &entry.plugin_id, entry.occurrence) else {
        return reject(
            request,
            RpcError::not_found(format!(
                "plugin {:?} vanished from track {} between lookup and set",
                entry.plugin_id, t.id
            )),
        );
    };

    let task = super::run_via_update(
        app,
        Message::Plugin(PluginMessage::SetPluginParam(
            instance_id,
            param.id,
            params.value,
        )),
    );
    (ack(app, request), task)
}

/// The engine instance id of the `occurrence`-th plugin with this CLAP
/// id on the track.
fn instance_for(
    t: &TrackState,
    plugin_id: &str,
    occurrence: u32,
) -> Option<resonance_audio::types::PluginInstanceId> {
    t.plugins
        .iter()
        .filter(|p| p.clap_plugin_id == plugin_id)
        .nth(occurrence as usize)
        .map(|p| p.instance_id)
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

/// `mixer.set_volume_db` — the same fader as [`set_volume`], in dB (ba
/// doc #273).
///
/// `TrackState.volume` is already stored in dB, so this dispatches the
/// caller's value verbatim: no `log10` on the way in, no `powf` on the
/// way out. Out-of-range values are rejected rather than clamped —
/// silently moving a level the caller asked for is how a mix ends up
/// subtly wrong with nothing to point at.
fn set_volume_db(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetVolumeDbParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_track(app, params.track_id.0).is_none() {
        return not_found_track(request, params.track_id.0);
    }
    if !params.volume_db.is_finite() {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "volume_db must be finite (got {}); the app's silence floor is {} dB, \
                 not -inf",
                params.volume_db,
                mixer::VOLUME_DB_MIN
            )),
        );
    }
    if !(mixer::VOLUME_DB_MIN..=mixer::VOLUME_DB_MAX).contains(&params.volume_db) {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "volume_db must be within {}..={} dB — the range the mixer fader spans \
                 ({} dB is silence, 0 dB is unity) — got {}",
                mixer::VOLUME_DB_MIN,
                mixer::VOLUME_DB_MAX,
                mixer::VOLUME_DB_MIN,
                params.volume_db
            )),
        );
    }
    let task = super::run_via_update(
        app,
        Message::Track(TrackMessage::SetTrackVolume(
            params.track_id.0,
            params.volume_db,
        )),
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
