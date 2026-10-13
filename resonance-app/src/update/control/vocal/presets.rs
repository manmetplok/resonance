//! `vocal.voice_presets` / `vocal.apply_voice_preset` /
//! `vocal.save_voice_preset` (`docs/features/compose/voice-presets.md`).
//!
//! - `voice_presets` lists the built-ins and the user's presets. It reads
//!   the machine, not the project, so it bypasses the mutation gate.
//! - `apply_voice_preset` is a lane edit: the lane inspector's own
//!   `ApplyVoicePreset` message through `run_via_update`, so one undo
//!   entry, one revision bump, and the lane reads `stale` until
//!   `vocal.render` (control edits never render). A preset whose bank is
//!   not installed still applies; the result says so.
//! - `save_voice_preset` reads the open project's lane and writes the
//!   user's preset folder: gated (it needs the lane) but listed in
//!   `reads_open_project_only`, and like every library write it records no
//!   undo entry and bumps no revision.

use iced::Task;
use resonance_control::methods::vocal as proto;
use resonance_control::{Request, Response, RpcError};

use super::super::reply::reject;
use super::super::{success, view_model};
use crate::compose::messages::LaneInspectorMsg;
use crate::compose::vocal_svs::voice_presets;
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::Resonance;

/// `vocal.voice_presets`: every preset, picker order.
pub(in crate::update::control) fn list(
    app: &Resonance,
    request: &Request,
) -> (Response, Task<Message>) {
    let presets = voice_presets::all(&app.voice_presets.user)
        .map(|p| view_model::voice_preset_view(p, &app.voicebanks))
        .collect();
    (
        success(request, &proto::VoicePresetList { presets }),
        Task::none(),
    )
}

/// `vocal.apply_voice_preset`: set the lane's voice and delivery.
pub(in crate::update::control) fn apply(
    app: &mut Resonance,
    request: &Request,
) -> (Response, Task<Message>) {
    let params: proto::ApplyVoicePresetParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let track_id: u64 = params.track_id.into();
    let definition_id = match super::resolve_vocal_lane(app, track_id, params.section_id) {
        Ok(id) => id,
        Err(e) => return reject(request, e),
    };
    let Some(preset) = voice_presets::find(&app.voice_presets.user, &params.preset).cloned() else {
        let known: Vec<&str> = voice_presets::all(&app.voice_presets.user)
            .map(|p| p.id.as_str())
            .collect();
        return reject(
            request,
            RpcError::not_found(format!(
                "no voice preset {:?} (vocal_voice_presets lists them: {})",
                params.preset,
                known.join(", ")
            )),
        );
    };
    let task = super::super::run_via_update(
        app,
        Message::Compose(ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg: LaneInspectorMsg::ApplyVoicePreset(Box::new(preset.settings.clone())),
        }),
    );
    let Some(lane) = super::params_result(app, definition_id, track_id) else {
        return reject(request, RpcError::internal("the vocal lane vanished"));
    };
    let missing = super::lane_bank_missing(app, definition_id, track_id);
    let result = proto::ApplyVoicePresetResult {
        track_id: lane.track_id,
        section_id: lane.section_id,
        preset_id: preset.id,
        preset_name: preset.name,
        params: lane.params,
        installed: missing.is_none(),
        warning: missing.map(|m| m.to_string()),
        revision: app.revision(),
    };
    (success(request, &result), task)
}

/// `vocal.save_voice_preset`: capture the lane's voice and delivery as a
/// user preset.
pub(in crate::update::control) fn save(
    app: &mut Resonance,
    request: &Request,
) -> (Response, Task<Message>) {
    let params: proto::SaveVoicePresetParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let track_id: u64 = params.track_id.into();
    let definition_id = match super::resolve_vocal_lane(app, track_id, params.section_id) {
        Ok(id) => id,
        Err(e) => return reject(request, e),
    };
    let name = params.name.trim().to_owned();
    if let Err(e) = crate::update::voice_presets::check_name(&name) {
        return reject(request, RpcError::invalid_params(e));
    }
    let replaced = app.voice_presets.store.exists(&name);
    if replaced && !params.overwrite {
        return reject(
            request,
            RpcError::needs_confirmation(format!(
                "a voice preset named {name:?} already exists; pass overwrite: true to replace \
                 it, or save under another name"
            )),
        );
    }
    let Some(lane) = crate::update::voice_presets::lane_params(app, definition_id, track_id) else {
        return reject(request, RpcError::internal("the vocal lane vanished"));
    };
    let preset = voice_presets::capture(
        &name,
        params.description.unwrap_or_default(),
        params.tags,
        &lane,
    );
    if let Err(e) = app.voice_presets.store.save(&preset) {
        return reject(request, RpcError::internal(e));
    }
    app.voice_presets.reload();
    app.refresh_vocal_choices();
    let result = proto::SaveVoicePresetResult {
        preset: view_model::voice_preset_view(&preset, &app.voicebanks),
        replaced,
    };
    (success(request, &result), Task::none())
}
