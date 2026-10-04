//! `midi_map.*` control methods — the project's MIDI Learn bindings
//! (doc #167, W1). Each write synthesizes the `MidiMapMessage` the GUI's
//! MIDI menu / Settings › MIDI sends and runs it through `update()`, so a
//! clear is one undo entry exactly like a click on its trash button.

use crate::message::{Message, MidiMapMessage};
use crate::update::control::plugin_target::{resolve_plugin_param, ChainOwner};
use crate::update::control::reply::{mutation_ack, no_track, reject, success};
use crate::update::control::run_via_update;
use crate::Resonance;
use iced::Task;
use resonance_common::{CcMode, ControlSource, MidiBinding, MidiTarget, SendId, TransportAction};
use resonance_control::ids::TrackId;
use resonance_control::methods::midi_map::{
    self as wire, BindingsResult, CancelLearnResult, ClearParams, ClearResult, LearnParams,
    LearnResult, MidiBindingView, MidiControl, MidiControlSource, MidiTargetSpec, MidiTargetView,
    MidiTransport,
};
use resonance_control::{Request, Response, RpcError};

/// Handle a `midi_map.*` request, or `None` for another namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let out = match request.method.as_str() {
        wire::BINDINGS => (success(request, &bindings(app)), Task::none()),
        wire::LEARN => learn(app, request),
        wire::CANCEL_LEARN => cancel_learn(app, request),
        wire::CLEAR => clear(app, request),
        wire::CLEAR_ALL => clear_all(app, request),
        _ => return None,
    };
    Some(out)
}

fn bindings(app: &Resonance) -> BindingsResult {
    let map = &app.devices.midi_map;
    BindingsResult {
        bindings: map.sorted().iter().map(|b| binding_view(app, b)).collect(),
        learning: map.learn_target.map(|t| target_view(app, t)),
        control_surface_input: app.settings.midi.control_surface_input.clone(),
    }
}

fn binding_view(app: &Resonance, b: &MidiBinding) -> MidiBindingView {
    let source = match b.source {
        ControlSource::Cc { channel, cc, mode } => MidiControlSource {
            kind: "cc".into(),
            channel: channel + 1,
            number: cc,
            mode: Some(match mode {
                CcMode::Absolute => "absolute".into(),
                CcMode::Relative(_) => "relative".into(),
            }),
        },
        ControlSource::Note { channel, note } => MidiControlSource {
            kind: "note".into(),
            channel: channel + 1,
            number: note,
            mode: None,
        },
    };
    MidiBindingView {
        id: b.id.0,
        source,
        target: target_view(app, b.target),
    }
}

/// A model target as the wire reads it back.
fn target_view(app: &Resonance, target: MidiTarget) -> MidiTargetView {
    let on = |id: u64| MidiTargetSpec {
        track_id: Some(TrackId(id)),
        ..Default::default()
    };
    let spec = match target {
        MidiTarget::TrackVolume(id) => MidiTargetSpec { control: Some(MidiControl::Volume), ..on(id) },
        MidiTarget::TrackPan(id) => MidiTargetSpec { control: Some(MidiControl::Pan), ..on(id) },
        MidiTarget::TrackMute(id) => MidiTargetSpec { control: Some(MidiControl::Mute), ..on(id) },
        MidiTarget::TrackSolo(id) => MidiTargetSpec { control: Some(MidiControl::Solo), ..on(id) },
        MidiTarget::SendLevel { track, send } => MidiTargetSpec {
            send_id: Some(send.0),
            ..on(track)
        },
        MidiTarget::PluginParam { instance, param_id } => {
            // The owning track, the plugin's CLAP id and its occurrence
            // among same-id plugins on that chain.
            let found = app.registry.tracks.iter().find_map(|t| {
                let slot = t.plugins.iter().find(|p| p.instance_id == instance)?;
                let occurrence = t
                    .plugins
                    .iter()
                    .take_while(|p| p.instance_id != instance)
                    .filter(|p| p.clap_plugin_id == slot.clap_plugin_id)
                    .count() as u32;
                Some((t.id, slot.clap_plugin_id.clone(), occurrence))
            });
            match found {
                Some((track, plugin_id, occurrence)) => MidiTargetSpec {
                    param: Some(param_id.to_string()),
                    plugin_id: Some(plugin_id),
                    occurrence: Some(occurrence),
                    ..on(track)
                },
                None => MidiTargetSpec {
                    param: Some(param_id.to_string()),
                    ..Default::default()
                },
            }
        }
        MidiTarget::Transport(action) => MidiTargetSpec {
            transport: Some(match action {
                TransportAction::Play => MidiTransport::Play,
                TransportAction::Stop => MidiTransport::Stop,
                TransportAction::Record => MidiTransport::Record,
                TransportAction::LoopToggle => MidiTransport::Loop,
            }),
            ..Default::default()
        },
    };
    MidiTargetView {
        spec,
        label: crate::view::midi_learn::target_label(app, target),
    }
}

/// Resolve a wire target against the open project.
fn resolve(app: &Resonance, spec: &MidiTargetSpec) -> Result<MidiTarget, RpcError> {
    if let Some(action) = spec.transport {
        if spec.track_id.is_some()
            || spec.control.is_some()
            || spec.send_id.is_some()
            || spec.param.is_some()
        {
            return Err(RpcError::invalid_params(
                "a transport target takes no track_id, control, send_id or param",
            ));
        }
        return Ok(MidiTarget::Transport(match action {
            MidiTransport::Play => TransportAction::Play,
            MidiTransport::Stop => TransportAction::Stop,
            MidiTransport::Record => TransportAction::Record,
            MidiTransport::Loop => TransportAction::LoopToggle,
        }));
    }
    let Some(track) = spec.track_id.map(|t| t.0) else {
        return Err(RpcError::invalid_params(
            "name the target: track_id with control / send_id / param, or transport",
        ));
    };
    if !app.registry.tracks.iter().any(|t| t.id == track) {
        return Err(no_track(track));
    }
    let named = usize::from(spec.control.is_some())
        + usize::from(spec.send_id.is_some())
        + usize::from(spec.param.is_some());
    if named != 1 {
        return Err(RpcError::invalid_params(
            "with track_id give exactly one of control (volume / pan / mute / solo), send_id \
             or param",
        ));
    }
    if let Some(control) = spec.control {
        return Ok(match control {
            MidiControl::Volume => MidiTarget::TrackVolume(track),
            MidiControl::Pan => MidiTarget::TrackPan(track),
            MidiControl::Mute => MidiTarget::TrackMute(track),
            MidiControl::Solo => MidiTarget::TrackSolo(track),
        });
    }
    if let Some(send) = spec.send_id {
        let ours = app.aux.sends.iter().any(|s| {
            s.id == send && matches!(s.source, resonance_audio::types::SendSource::Track(t) if t == track)
        });
        if !ours {
            return Err(RpcError::not_found(format!(
                "track {track} has no send {send}; see track_sends"
            )));
        }
        return Ok(MidiTarget::SendLevel {
            track,
            send: SendId(send),
        });
    }
    let param = spec.param.as_deref().unwrap_or_default();
    let (plugin, param) = resolve_plugin_param(
        app,
        ChainOwner::Track(track),
        spec.plugin_id.as_deref(),
        spec.occurrence,
        param,
    )?;
    Ok(MidiTarget::PluginParam {
        instance: plugin.instance_id,
        param_id: param.id,
    })
}

fn learn(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: LearnParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let target = match resolve(app, &params.target) {
        Ok(t) => t,
        Err(e) => return reject(request, e),
    };
    // `Learn` on the already-armed target disarms (the GUI toggle); an
    // agent asking twice means "armed", so it is a no-op then.
    let task = if app.devices.midi_map.learn_target == Some(target) {
        Task::none()
    } else {
        run_via_update(app, Message::MidiMap(MidiMapMessage::Learn(target)))
    };
    let result = LearnResult {
        learning: target_view(app, target),
        revision: mutation_ack(app).revision,
    };
    (success(request, &result), task)
}

fn cancel_learn(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let cancelled = app.devices.midi_map.learn_target.is_some();
    let task = run_via_update(app, Message::MidiMap(MidiMapMessage::CancelLearn));
    let result = CancelLearnResult {
        cancelled,
        revision: mutation_ack(app).revision,
    };
    (success(request, &result), task)
}

fn clear(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: ClearParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let id = resonance_common::BindingId(params.id);
    if !app.devices.midi_map.bindings.contains_key(&id) {
        return reject(
            request,
            RpcError::not_found(format!("no MIDI binding {}; see midi_map_bindings", params.id)),
        );
    }
    let task = run_via_update(app, Message::MidiMap(MidiMapMessage::Clear(id)));
    let result = ClearResult {
        cleared: 1,
        revision: mutation_ack(app).revision,
    };
    (success(request, &result), task)
}

fn clear_all(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let cleared = app.devices.midi_map.bindings.len() as u32;
    // Nothing to clear: no edit, no undo entry, no revision.
    let task = if cleared == 0 {
        Task::none()
    } else {
        run_via_update(app, Message::MidiMap(MidiMapMessage::ClearAll))
    };
    let result = ClearResult {
        cleared,
        revision: mutation_ack(app).revision,
    };
    (success(request, &result), task)
}
