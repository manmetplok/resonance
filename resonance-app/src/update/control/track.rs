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

use crate::message::{Message, MixerMessage, PluginMessage, TrackMessage};
use crate::state::TrackState;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::mixer::{
    self, SetMuteParams, SetPanParams, SetSoloParams, SetVolumeDbParams, SetVolumeParams,
};
use resonance_control::methods::track::{
    self, AddParams, AddPluginParams, AddResult, DeleteParams, RemoveEffectParams, RenameParams,
};
use resonance_audio::types::{aux_send_would_cycle, SendSource, TrackOutput};
use resonance_control::{
    MutationAck, Request, Response, RpcError, TrackKind, TrackOutput as WireTrackOutput,
};

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
        track::SET_OUTPUT => set_output(app, request),
        track::ADD_SEND => add_send(app, request),
        track::SET_SEND => set_send(app, request),
        track::REMOVE_SEND => remove_send(app, request),
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
    let task = super::run_via_update(
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
    (super::success(request, &result), task)
}

// ---------------------------------------------------------------------------
// track.set_output
// ---------------------------------------------------------------------------

/// `track.set_output` — send a track's post-fader audio to master, or
/// through a group bus first (ba doc #273, todo #1228).
///
/// `song.summary` / `song.tracks` report the current destination in the
/// same shape, so read and write share one vocabulary.
fn set_output(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: track::SetOutputParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(t) = find_track(app, params.track_id.0) else {
        return not_found_track(request, params.track_id.0);
    };
    // A bus is a track id in the same space, so "route the bus into
    // itself" is expressible; the engine models bus -> master only.
    if app.registry.busses.iter().any(|b| b.id == params.track_id.0) {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "{} is a bus, not a track; busses always feed master and cannot be re-routed",
                params.track_id
            )),
        );
    }

    let output = match params.output {
        WireTrackOutput::Master => TrackOutput::Master,
        WireTrackOutput::Bus(bus_id) => {
            if !app.registry.busses.iter().any(|b| b.id == bus_id.0) {
                let known: Vec<String> = app
                    .registry
                    .busses
                    .iter()
                    .map(|b| format!("{} ({})", b.id, b.name))
                    .collect();
                return reject(
                    request,
                    RpcError::not_found(format!(
                        "no bus with id {bus_id}; create one with bus.create. Existing busses: \
                         [{}]",
                        known.join(", ")
                    )),
                );
            }
            TrackOutput::Bus(bus_id.0)
        }
    };

    // Idempotent: re-routing somewhere it already goes records no undo
    // entry and sends no engine command.
    if t.output == output {
        return (ack(app, request), Task::none());
    }
    let task = super::run_via_update(
        app,
        Message::Track(TrackMessage::SetTrackOutput(params.track_id.0, output)),
    );
    (ack(app, request), task)
}

// ---------------------------------------------------------------------------
// track.add_send / set_send / remove_send (ba doc #273, todo #1229)
// ---------------------------------------------------------------------------
//
// The engine is the single writer of the send graph: these handlers
// synthesize the existing `MixerMessage` values and let `AuxSendChanged`
// / `AuxSendRemoved` update `AuxSendState`. Cyclic routes are validated
// with the engine's own `aux_send_would_cycle` predicate (not a second
// implementation) so a route the engine would refuse is reported as
// `invalid_params` here instead of silently vanishing.
//
// NOTE: aux sends are NOT persisted — nothing in
// `update/project_io/serialize.rs` writes the send graph. That is open
// ba todo #482; until it lands a send created here is lost on
// save + reload, and the MCP tool descriptions say so.

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
fn add_send(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
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
    let role_task = super::run_via_update(
        app,
        Message::Mixer(MixerMessage::SetBusReturnRole(params.to_bus.0, true)),
    );
    let send_task = super::run_via_update(
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
        super::success(request, &result),
        Task::batch([role_task, send_task]),
    )
}

/// `track.set_send` — change level / tap point / enable / destination.
/// Omitted fields keep their current value.
fn set_send(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
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
            tasks.push(super::run_via_update(
                app,
                Message::Mixer(MixerMessage::SetSendDest(send.id, to_bus.0)),
            ));
        }
    }
    if let Some(db) = params.level_db {
        if send.level_db != db {
            tasks.push(super::run_via_update(
                app,
                Message::Mixer(MixerMessage::SetSendLevel(send.id, db)),
            ));
        }
    }
    if let Some(pre) = params.pre_fader {
        if send.pre_fader != pre {
            tasks.push(super::run_via_update(
                app,
                Message::Mixer(MixerMessage::ToggleSendPreFader(send.id)),
            ));
        }
    }
    if let Some(enabled) = params.enabled {
        if send.enabled != enabled {
            tasks.push(super::run_via_update(
                app,
                Message::Mixer(MixerMessage::ToggleSendEnabled(send.id)),
            ));
        }
    }
    (ack(app, request), Task::batch(tasks))
}

/// `track.remove_send` — delete a send outright.
fn remove_send(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: track::RemoveSendParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_send(app, params.send_id.0).is_none() {
        return not_found_send(request, params.send_id.0);
    }
    let task = super::run_via_update(
        app,
        Message::Mixer(MixerMessage::RemoveSend(params.send_id.0)),
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

    // The one window `track.add_effect`'s synchronous commit (todo
    // #1234) cannot close: the slot exists, but its parameter list only
    // arrives with the engine's `PluginAdded` echo. Say that, rather
    // than falling through to "plugin X has no parameter Y (has: [])" —
    // which is what made a working add look like a failed one.
    if entry.params.is_empty() {
        return reject(
            request,
            RpcError::busy(format!(
                "plugin {:?} is on track {} but is still initializing — its parameter list \
                 arrives with the engine echo, usually within a frame. Retry, or read \
                 track.plugin_params until its params array is non-empty. (A plugin that \
                 genuinely exposes no parameters reports the same empty list.)",
                entry.plugin_id, t.id
            )),
        );
    }

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
    //
    // ...but the bounds themselves are f64 renderings of f32 plugin
    // declarations, so an exact comparison rejects the very number this
    // API just reported. nih-plug declares the compressor's attack as
    // `FloatRange::Skewed { min: 0.1, .. }` in f32; widened to f64 that
    // is 0.10000000149011612, which is strictly greater than the f64 0.1
    // a caller types. Every f32-declared bound without an exact binary
    // representation has this, on every plugin (ba doc #273, todo
    // #1235). So compare with an f32-precision tolerance and CLAMP what
    // lands inside the band — the DSP never sees a value below the
    // plugin's real minimum, and a genuinely out-of-range request is
    // still refused with the same message it always got.
    let value = match clamp_within_tolerance(params.value, param.min, param.max) {
        Some(value) => value,
        None => {
            return reject(
                request,
                RpcError::invalid_params(format!(
                    "{} must be within {}..={} (got {})",
                    param.name, param.min, param.max, params.value
                )),
            )
        }
    };

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
        Message::Plugin(PluginMessage::SetPluginParam(instance_id, param.id, value)),
    );
    (ack(app, request), task)
}

/// `value` clamped into `min..=max`, or `None` when it lies genuinely
/// outside the range (ba doc #273, todo #1235).
///
/// "Genuinely" is the whole point: `min`/`max` reach the wire as f64
/// widenings of f32 plugin declarations, so the exact f64 a caller reads
/// out of `track.plugin_params` round-trips, but the tidy decimal it
/// *means* (`0.1`) sits a few ULPs outside. The tolerance scales with the
/// magnitude of the range rather than being a fixed absolute epsilon —
/// on a 20..20000 Hz parameter an absolute 1e-7 would be meaningless,
/// and on a 0..1 parameter it would be far too generous.
///
/// A request beyond the tolerance band is still `None`, so
/// `set_plugin_param` keeps its reject-don't-clamp promise for values
/// the caller really did get wrong.
pub(crate) fn clamp_within_tolerance(value: f64, min: f64, max: f64) -> Option<f64> {
    if !(min <= max) {
        // A plugin declaring an inverted range is a plugin bug; take the
        // value as-is rather than rejecting everything it exposes.
        //
        // Written as `!(min <= max)` rather than `min > max` so a NaN
        // bound lands here too: NaN fails every comparison, so `min > max`
        // would wave it through to `value.clamp(min, max)`, and
        // `f64::clamp` *asserts* `min <= max` — a third-party CLAP plugin
        // reporting a NaN bound would panic the update loop. `ParamInfo`
        // takes `min_value`/`max_value` straight from the plugin with no
        // sanitisation (`resonance-audio/src/clap_host/instance.rs`), so
        // that is reachable from outside the project.
        return Some(value);
    }
    let tol = (max - min)
        .abs()
        .max(max.abs())
        .max(min.abs())
        .max(1.0)
        * f32::EPSILON as f64;
    if value < min - tol || value > max + tol {
        return None;
    }
    Some(value.clamp(min, max))
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
