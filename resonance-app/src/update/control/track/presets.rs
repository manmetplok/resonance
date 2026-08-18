//! `track.plugin_presets` / `.load_plugin_preset` / `.save_plugin_preset`
//! — the control half of the shared preset bar every plugin editor now
//! carries (ba todo #1333, finding X1).
//!
//! Addressing is `track.plugin_params`'s: a track id, an optional CLAP id
//! (omitted means the track's instrument) and an `occurrence` for a track
//! carrying the same plugin twice. Everything past that is
//! [`crate::update::control::plugin_presets`], shared with the bus and
//! master surfaces.

use super::{ack, find_track, instance_for, not_found_track, reject};
use crate::message::Message;
use crate::update::control::{plugin_presets, run_via_update, success, view_model};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::track;
use resonance_control::{Request, Response, RpcError};

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

    app.pending_plugin_preset_save = Some(crate::PendingPluginPresetSave {
        instance_id,
        clap_id,
        name: params.name.trim().to_string(),
    });
    let _ = app
        .engine
        .send(resonance_audio::types::AudioCommand::SavePluginState { instance_id });

    (ack(app, request), Task::none())
}
