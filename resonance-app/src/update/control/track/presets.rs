//! The two preset surfaces a track has, and they are not the same thing.
//!
//! `track.save_preset` / `.presets` / `.apply_preset` capture a whole
//! dialled-in track and stamp it out again (ba todo #1303, finding P1),
//! while `track.plugin_presets` / `.load_plugin_preset` /
//! `.save_plugin_preset` are the control half of the shared preset bar
//! every plugin editor carries (ba todo #1333, finding X1) and address
//! one plugin inside the chain.
//!
//! Plugin-preset addressing is `track.plugin_params`'s: a track id, an
//! optional CLAP id (omitted means the track's instrument) and an
//! `occurrence` for a track carrying the same plugin twice. Everything
//! past that is [`crate::update::control::plugin_presets`], shared with
//! the bus and master surfaces.
//!
//! The track-preset capture pipeline had always worked and nothing ever
//! started it: `pending_preset_save` was declared, initialised to `None`
//! and taken on the engine's `AllPluginStatesSaved` echo, with no code
//! anywhere setting it. So the app could apply and delete user presets it
//! had no way to create — the menu listed only presets someone had
//! written as JSON by hand. `track.save_preset` is the control half of
//! the GUI's "Save as preset…"; both surfaces raise the same
//! `TrackMessage`, so the overwrite rule and the captured contents cannot
//! differ between them.

use super::{ack, find_track, frozen_reject, instance_for, not_found_track, reject};
use crate::message::{Message, TrackMessage};
use crate::update::control::{plugin_presets, run_via_update, success, view_model};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::track;
use resonance_control::methods::track::{
    AddResult, ApplyPresetParams, PresetView, PresetsView, SavePresetParams,
};
use resonance_control::{Request, Response, RpcError, TrackKind};

/// Resolve the addressed plugin to `(clap id, occurrence, params, instance)`.
///
/// Shared by all three methods so they cannot disagree about which plugin
/// a given request means.
fn resolve<'a>(
    app: &'a Resonance,
    request: &Request,
    track_id: u64,
    plugin_id: &Option<String>,
    occurrence: Option<u32>,
) -> Result<
    (
        String,
        Vec<track::PluginParamView>,
        resonance_audio::types::PluginInstanceId,
    ),
    Response,
> {
    let Some(t) = find_track(app, track_id).cloned() else {
        return Err(not_found_track(request, track_id).0);
    };
    let entries = view_model::plugin_entries(app, &t);
    let occurrence = occurrence.unwrap_or(0);
    let entry = match plugin_id {
        Some(id) => entries
            .iter()
            .find(|e| &e.plugin_id == id && e.occurrence == occurrence),
        None => entries
            .iter()
            .find(|e| e.kind == track::PluginKind::Instrument),
    };
    let Some(entry) = entry else {
        let error = match plugin_id {
            Some(id) => view_model::unknown_plugin_on_track(app, &t, id, occurrence),
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
        return Err(reject(request, error).0);
    };
    let Some(instance_id) = instance_for(&t, &entry.plugin_id, entry.occurrence) else {
        return Err(reject(
            request,
            RpcError::not_found(format!(
                "plugin {:?} vanished from track {} between lookup and use",
                entry.plugin_id, t.id
            )),
        )
        .0);
    };
    Ok((entry.plugin_id.clone(), entry.params.clone(), instance_id))
}

/// `track.plugin_presets` — what this plugin can recall. Read-only.
pub(super) fn plugin_presets(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: track::PluginPresetsParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let (clap_id, _, _) = match resolve(
        app,
        request,
        params.track_id.0,
        &params.plugin_id,
        params.occurrence,
    ) {
        Ok(resolved) => resolved,
        Err(response) => return (response, Task::none()),
    };
    (
        success(request, &plugin_presets_view(app, &clap_id)),
        Task::none(),
    )
}

fn plugin_presets_view(
    app: &Resonance,
    clap_id: &str,
) -> resonance_control::methods::plugin_preset::PluginPresetsView {
    plugin_presets::view(app, clap_id)
}

/// `track.load_plugin_preset` — recall a preset, as one undoable edit.
pub(super) fn load_plugin_preset(
    app: &mut Resonance,
    request: &Request,
) -> (Response, Task<Message>) {
    let params: track::LoadPluginPresetParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let (clap_id, mirrored, instance_id) = match resolve(
        app,
        request,
        params.track_id.0,
        &params.plugin_id,
        params.occurrence,
    ) {
        Ok(resolved) => resolved,
        Err(response) => return (response, Task::none()),
    };
    // A preset recall is a frozen-input edit (`LoadPluginPreset`,
    // gates.rs); reject rather than ack an edit the gate would swallow.
    if let Some(e) = frozen_reject(app, params.track_id.0) {
        return reject(request, e);
    }

    let message = match plugin_presets::load_message(
        app,
        &clap_id,
        instance_id,
        &mirrored,
        &params.preset,
        params.source,
    ) {
        Ok(message) => message,
        Err(e) => return reject(request, e),
    };
    let task = run_via_update(app, message);
    (ack(app, request), task)
}

/// `track.save_plugin_preset` — capture this plugin's current sound.
///
/// Answers as soon as the capture is armed, not when the file lands: the
/// plugin's state blob comes back from the engine a beat later and only
/// then is the preset written. Read `track.plugin_presets` to see it
/// appear — the same one-cycle gap `track.add_effect` has for its
/// parameter list, and `track.save_preset` for its own capture.
pub(super) fn save_plugin_preset(
    app: &mut Resonance,
    request: &Request,
) -> (Response, Task<Message>) {
    let params: track::SavePluginPresetParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let (clap_id, _, instance_id) = match resolve(
        app,
        request,
        params.track_id.0,
        &params.plugin_id,
        params.occurrence,
    ) {
        Ok(resolved) => resolved,
        Err(response) => return (response, Task::none()),
    };

    if let Err(e) = plugin_presets::check_save(app, &clap_id, &params.name, params.overwrite) {
        return reject(request, e);
    }

    app.presets.pending_plugin_preset_save = Some(crate::PendingPluginPresetSave {
        instance_id,
        clap_id,
        name: params.name.trim().to_string(),
    });
    let _ = app
        .engine
        .send(resonance_audio::types::AudioCommand::SavePluginPresetState { instance_id });

    (ack(app, request), Task::none())
}

/// `track.save_preset` — capture a track as a reusable preset.
///
/// Answers as soon as the capture is armed, not when the file lands: the
/// plugins' opaque state blobs come back from the engine a beat later
/// and only then is the preset written (see
/// [`crate::PendingPresetSave`]). Read `track.presets` to see it appear;
/// that is the same one-cycle gap `track.add_effect` has for its
/// parameter list.
pub(super) fn save_preset(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SavePresetParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(track) = find_track(app, params.track_id.0) else {
        return super::not_found_track(request, params.track_id.0);
    };

    // Default to the track's own name — usually what the user would
    // type, and always visible afterwards in `track.presets`.
    let name = params
        .name
        .clone()
        .unwrap_or_else(|| track.name.clone())
        .trim()
        .to_string();
    if name.is_empty() {
        return reject(
            request,
            RpcError::invalid_params("name a preset before saving it"),
        );
    }

    // The destructive-operation convention: a preset is a file, and
    // saving over one loses whatever was in it. Refuse and say what
    // would be replaced, rather than replace it quietly.
    if !params.overwrite && crate::presets::user_preset_exists(&name) {
        return reject(
            request,
            RpcError::needs_confirmation(format!(
                "a preset named {name:?} already exists; pass overwrite: true to replace it, or \
                 save under another name"
            )),
        );
    }

    let task = run_via_update(
        app,
        Message::Track(TrackMessage::SaveTrackAsPreset {
            track_id: params.track_id.0,
            name,
            // Already checked above, with a better error than the
            // handler's own.
            overwrite: true,
        }),
    );
    (super::ack(app, request), task)
}

/// `track.presets` — the preset library.
///
/// Read-only, and deliberately lists BOTH sets: the built-ins that ship
/// with the app (mixer settings and an instrument identity, no chain)
/// and the user's own saves (usually a chain with its state). `builtin`
/// is what tells them apart, and `plugins` is what says whether recalling
/// one will bring a sound with it.
pub(super) fn presets(app: &Resonance, request: &Request) -> Response {
    let view = |preset: &crate::presets::TrackPreset, builtin: bool| PresetView {
        name: preset.name.clone(),
        kind: preset_kind(preset),
        builtin,
        plugins: preset
            .plugins
            .iter()
            .map(|p| p.clap_plugin_id.clone())
            .collect(),
    };
    let presets: Vec<PresetView> = app
        .presets.default_presets
        .iter()
        .map(|p| view(p, true))
        .chain(app.presets.user_presets.iter().map(|p| view(p, false)))
        .collect();
    success(request, &PresetsView { presets })
}

/// `track.apply_preset` — create a track from a preset.
///
/// Applying CREATES rather than overwrites (the same thing picking a
/// preset in the add-track menu does), so there is nothing to confirm.
/// The track id comes back immediately because the app allocates it and
/// hints it to the engine — the preset's chain arrives with the engine's
/// echo, exactly as it does for a preset picked in the GUI.
pub(super) fn apply_preset(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: ApplyPresetParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let wanted = params.preset.trim();
    let preset = app
        .presets.default_presets
        .iter()
        .chain(app.presets.user_presets.iter())
        .find(|p| p.name.eq_ignore_ascii_case(wanted))
        .cloned();
    let Some(preset) = preset else {
        let known: Vec<&str> = app
            .presets.default_presets
            .iter()
            .chain(app.presets.user_presets.iter())
            .map(|p| p.name.as_str())
            .collect();
        return reject(
            request,
            RpcError::not_found(format!(
                "no preset named {wanted:?} (have: [{}])",
                known.join(", ")
            )),
        );
    };

    let track_id = app.allocate_track_id();
    let task = run_via_update(
        app,
        Message::Track(TrackMessage::AddTrackFromPreset {
            preset: Box::new(preset),
            id_hint: Some(track_id),
            name: params.name.clone(),
        }),
    );
    let result = AddResult {
        track_id: resonance_control::ids::TrackId(track_id),
        revision: app.revision(),
    };
    (success(request, &result), task)
}

/// The wire kind a preset makes. Presets store the track type as the
/// string the app writes into its JSON, so this is the one place that
/// mapping lives.
fn preset_kind(preset: &crate::presets::TrackPreset) -> TrackKind {
    match preset.track_type.as_str() {
        "instrument" => TrackKind::Instrument,
        "vocal" => TrackKind::Vocal,
        "audio" => TrackKind::Audio,
        _ => TrackKind::Unknown,
    }
}
