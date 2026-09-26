//! `external.*` control methods — external-instrument tracks (doc #169).
//!
//! Every mutation synthesizes the existing `ExternalInstrumentMessage`
//! (the same values the inspector emits) and routes it through the FULL
//! `update()` path via [`super::run_via_update`], so a remote edit is
//! undoable exactly like a manual one. `external.devices` and
//! `external.status` are read-only.
//!
//! Two conventions worth knowing:
//!
//! - The setters take `Option<Option<T>>`: an absent field leaves that
//!   half alone, an explicit `null` clears it. Picking a MIDI channel
//!   without disturbing the device is the common case, and a single
//!   `Option` cannot express it.
//! - Monitor and record-arm are app-side *toggles*; the control API is
//!   declarative (`enabled: true`), so those handlers read the current
//!   value and dispatch the toggle only when it differs. Setting a flag
//!   to what it already is is a no-op, not a flip.

use crate::message::{ExternalInstrumentMessage as Ext, Message, TrackMessage};
use crate::state::TrackState;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::external::{
    self, AudioInputView, BounceParams, DevicesView, ExternalTrackView, PlaybackSource,
    SetLatencyParams, SetMidiOutParams, SetMonitorParams, SetPatchParams, SetPlaybackSourceParams,
    SetRecordArmParams, SetReturnParams, StatusParams, StatusView, TrackParams,
};
use resonance_control::{Request, Response, RpcError};

use super::reply::{ack, no_track, reject};

/// Handle an `external.*` request, or `None` when `method` belongs to
/// another namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let out = match request.method.as_str() {
        external::DEVICES => (devices(app, request), Task::none()),
        external::STATUS => (status(app, request), Task::none()),
        external::ENABLE => enable(app, request),
        external::DISABLE => disable(app, request),
        external::SET_MIDI_OUT => set_midi_out(app, request),
        external::SET_RETURN => set_return(app, request),
        external::SET_PATCH => set_patch(app, request),
        external::SET_LATENCY => set_latency(app, request),
        external::DETECT_LATENCY => detect_latency(app, request),
        external::SET_MONITOR => set_monitor(app, request),
        external::SET_RECORD_ARM => set_record_arm(app, request),
        external::SET_PLAYBACK_SOURCE => set_playback_source(app, request),
        external::BOUNCE => bounce(app, request),
        _ => return None,
    };
    Some(out)
}

fn find_track(app: &Resonance, id: u64) -> Option<&TrackState> {
    app.registry.tracks.iter().find(|t| t.id == id)
}

/// Resolve a track that must already be in external-instrument mode.
/// Separating "no such track" from "not external" matters: the second is
/// fixable with `external.enable`, and saying so saves a round trip.
fn require_external(app: &Resonance, id: u64) -> Result<&TrackState, RpcError> {
    let track = find_track(app, id).ok_or_else(|| no_track(id))?;
    if !app.external_instruments.contains_key(&id) {
        return Err(RpcError::invalid_params(format!(
            "track {id} is not an external instrument; call external.enable first"
        )));
    }
    Ok(track)
}

// ---------------------------------------------------------------------------
// Read-only views
// ---------------------------------------------------------------------------

fn devices(app: &Resonance, request: &Request) -> Response {
    let view = DevicesView {
        midi_outputs: app
            .midi_devices
            .midi_output_devices
            .iter()
            .map(|d| d.name.clone())
            .collect(),
        audio_inputs: app
            .input_devices
            .iter()
            .map(|d| AudioInputView {
                name: d.name.clone(),
                channels: d.channels,
                default: app.default_input_device_name.as_deref() == Some(d.name.as_str()),
            })
            .collect(),
    };
    super::success(request, &view)
}

fn status(app: &Resonance, request: &Request) -> Response {
    let params: StatusParams = match super::optional_params(request) {
        Ok(p) => p,
        Err(e) => return super::failure(request, e),
    };

    let wanted = params.track_id.map(|t| t.0);
    if let Some(id) = wanted {
        if find_track(app, id).is_none() {
            return super::failure(request, no_track(id));
        }
    }

    let tracks: Vec<ExternalTrackView> = app
        .registry
        .tracks
        .iter()
        .filter(|t| wanted.is_none_or(|id| t.id == id))
        .filter_map(|t| app.external_instruments.get(&t.id).map(|ext| (t, ext)))
        .map(|(t, ext)| ExternalTrackView {
            track_id: resonance_control::ids::TrackId(t.id),
            name: t.name.clone(),
            status: status_label(ext.status(t)),
            midi_out_device: t.midi_output_device.clone(),
            // The app stores the channel 0-based and shows it 1-based;
            // the wire follows the UI so a value read here can be handed
            // straight back to `set_midi_out`.
            midi_out_channel: t.midi_output_channel.unwrap_or(0) + 1,
            return_device: t.input_device_name.clone(),
            return_port: t.input_port_index,
            bank: ext.bank,
            program: ext.program,
            latency_offset_samples: ext.latency_offset_samples,
            latency_detect_in_progress: ext.latency_detect_in_progress,
            latency_detect_error: ext.latency_detect_error.clone(),
            monitor_enabled: t.monitor_enabled,
            record_armed: t.record_armed,
            playback_source: wire_source(t.playback_source),
            take_count: app.clips.iter().filter(|c| c.track_id == t.id).count(),
            midi_out_offline: ext.midi_out_offline,
            return_input_offline: ext.return_input_offline,
        })
        .collect();

    super::success(
        request,
        &StatusView {
            tracks,
            revision: app.revision(),
        },
    )
}

/// The lifecycle word `external.status` reports. `pub(super)` because
/// `song.summary` reports the same word for a track whose device
/// definition is unset, so the two views cannot disagree.
pub(super) fn status_label(status: crate::state::ExternalInstrumentStatus) -> String {
    use crate::state::ExternalInstrumentStatus as S;
    match status {
        S::Unconfigured => "unconfigured",
        S::Configuring => "configuring",
        S::Live => "live",
        S::Offline => "offline",
    }
    .to_string()
}

fn wire_source(source: resonance_common::PlaybackSource) -> PlaybackSource {
    match source {
        resonance_common::PlaybackSource::Live => PlaybackSource::Live,
        resonance_common::PlaybackSource::Recorded => PlaybackSource::Recorded,
    }
}

// ---------------------------------------------------------------------------
// Mode
// ---------------------------------------------------------------------------

fn enable(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: TrackParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let id = params.track_id.0;
    let Some(track) = find_track(app, id) else {
        return reject(request, no_track(id));
    };
    // A sub-track is fed by its parent's plugin fan-out and has no clips
    // or chain of its own, so there is nothing for a hardware route to
    // attach to.
    if track.sub_track.is_some() {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "track {id} is a sub-track; external mode applies to the parent track"
            )),
        );
    }
    let task = super::run_via_update(app, Message::ExternalInstrument(Ext::Enable(id)));
    (ack(app, request), task)
}

fn disable(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: TrackParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if let Err(e) = require_external(app, params.track_id.0) {
        return reject(request, e);
    }
    let task = super::run_via_update(
        app,
        Message::ExternalInstrument(Ext::Disable(params.track_id.0)),
    );
    (ack(app, request), task)
}

// ---------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------

fn set_midi_out(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetMidiOutParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let id = params.track_id.0;
    if let Err(e) = require_external(app, id) {
        return reject(request, e);
    }
    if params.device.is_none() && params.channel.is_none() {
        return reject(
            request,
            RpcError::invalid_params("set at least one of device / channel"),
        );
    }
    // An unknown port name would be stored and silently swallow every
    // note, so check it against the live list rather than accepting it.
    if let Some(Some(name)) = &params.device {
        if !app.midi_devices.midi_output_devices.iter().any(|d| &d.name == name) {
            return reject(
                request,
                RpcError::not_found(format!(
                    "no MIDI output named {name:?}; external.devices lists what is connected"
                )),
            );
        }
    }
    if let Some(channel) = params.channel {
        if !(1..=16).contains(&channel) {
            return reject(
                request,
                RpcError::invalid_params(format!("MIDI channel must be 1..=16, got {channel}")),
            );
        }
    }

    // Device + channel land as ONE undoable transaction: one revision
    // bump per call, one edit_undo to take the whole route change back.
    app.with_compound_undo(|app| {
        let mut task = Task::none();
        if let Some(device) = params.device {
            task = super::run_via_update(app, Message::ExternalInstrument(Ext::SetMidiOutDevice(id, device)));
        }
        if let Some(channel) = params.channel {
            // App-side channels are 0-based; the wire (like the UI) is 1-based.
            let task2 = super::run_via_update(
                app,
                Message::ExternalInstrument(Ext::SetMidiOutChannel(id, Some(channel - 1))),
            );
            task = Task::batch([task, task2]);
        }
        (ack(app, request), task)
    })
}

fn set_return(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetReturnParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let id = params.track_id.0;
    if let Err(e) = require_external(app, id) {
        return reject(request, e);
    }
    if params.device.is_none() && params.port.is_none() {
        return reject(
            request,
            RpcError::invalid_params("set at least one of device / port"),
        );
    }
    if let Some(Some(name)) = &params.device {
        if !app.input_devices.iter().any(|d| &d.name == name) {
            return reject(
                request,
                RpcError::not_found(format!(
                    "no audio input named {name:?}; external.devices lists what is connected"
                )),
            );
        }
    }
    // Validate the port against whichever device the track ends up on.
    if let Some(port) = params.port {
        let device_name = match &params.device {
            Some(Some(name)) => Some(name.clone()),
            Some(None) => None,
            None => find_track(app, id).and_then(|t| t.input_device_name.clone()),
        };
        if let Some(name) = device_name {
            if let Some(dev) = app.input_devices.iter().find(|d| d.name == name) {
                if dev.channels > 0 && port >= dev.channels {
                    return reject(
                        request,
                        RpcError::invalid_params(format!(
                            "port {port} is out of range for {name:?}, which has {} channel(s)",
                            dev.channels
                        )),
                    );
                }
            }
        }
    }

    // Device + port land as ONE undoable transaction, exactly like
    // `set_midi_out`'s device + channel.
    app.with_compound_undo(|app| {
        let mut task = Task::none();
        if let Some(device) = params.device {
            task = super::run_via_update(app, Message::ExternalInstrument(Ext::SetReturnDevice(id, device)));
        }
        if let Some(port) = params.port {
            let task2 = super::run_via_update(app, Message::ExternalInstrument(Ext::SetReturnPort(id, port)));
            task = Task::batch([task, task2]);
        }
        (ack(app, request), task)
    })
}

// ---------------------------------------------------------------------------
// Patch / latency
// ---------------------------------------------------------------------------

fn set_patch(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetPatchParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let id = params.track_id.0;
    if let Err(e) = require_external(app, id) {
        return reject(request, e);
    }
    if params.bank.is_none() && params.program.is_none() {
        return reject(
            request,
            RpcError::invalid_params("set at least one of bank / program"),
        );
    }
    if let Some(Some(bank)) = params.bank {
        if bank > 16_383 {
            return reject(
                request,
                RpcError::invalid_params(format!("bank is a 14-bit value (0..=16383), got {bank}")),
            );
        }
    }
    // `program` is a u8 on the wire, so 0..=127 is the only extra check
    // the MIDI spec adds.
    if let Some(Some(program)) = params.program {
        if program > 127 {
            return reject(
                request,
                RpcError::invalid_params(format!("program must be 0..=127, got {program}")),
            );
        }
    }

    // Send both halves as ONE patch change when both were given: two
    // separate messages would fire two Bank Select + Program Change
    // pairs at the synth, and the first would select a patch the caller
    // never asked for.
    let (bank, program) = {
        let ext = &app.external_instruments[&id];
        (
            params.bank.unwrap_or(ext.bank),
            params.program.unwrap_or(ext.program),
        )
    };
    let task = super::run_via_update(
        app,
        Message::ExternalInstrument(Ext::SetPatch(id, bank, program)),
    );
    (ack(app, request), task)
}

fn set_latency(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetLatencyParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let id = params.track_id.0;
    if let Err(e) = require_external(app, id) {
        return reject(request, e);
    }
    let task = super::run_via_update(
        app,
        Message::ExternalInstrument(Ext::SetLatencyOffset(id, params.offset_samples)),
    );
    (ack(app, request), task)
}

fn detect_latency(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: TrackParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let id = params.track_id.0;
    let track = match require_external(app, id) {
        Ok(t) => t,
        Err(e) => return reject(request, e),
    };
    // The engine fires a MIDI impulse and listens for it, so both halves
    // of the route have to exist and the transport has to be stopped.
    // The message itself no-ops in those cases; saying why is better.
    if track.midi_output_device.is_none() || track.input_device_name.is_none() {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "track {id} needs both a MIDI output and an audio return before its latency \
                 can be measured"
            )),
        );
    }
    if app.transport.playing {
        return reject(
            request,
            RpcError::invalid_params("stop the transport before measuring latency"),
        );
    }
    if app.external_instruments[&id].latency_detect_in_progress {
        return reject(
            request,
            RpcError::invalid_params(format!("a latency measurement is already running on track {id}")),
        );
    }
    let task = super::run_via_update(app, Message::ExternalInstrument(Ext::DetectLatency(id)));
    (ack(app, request), task)
}

// ---------------------------------------------------------------------------
// Monitoring / recording / playback
// ---------------------------------------------------------------------------

fn set_monitor(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetMonitorParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let id = params.track_id.0;
    let track = match require_external(app, id) {
        Ok(t) => t,
        Err(e) => return reject(request, e),
    };
    if track.monitor_enabled == params.enabled {
        return (ack(app, request), Task::none());
    }
    let task = super::run_via_update(app, Message::ExternalInstrument(Ext::ToggleMonitor(id)));
    (ack(app, request), task)
}

fn set_record_arm(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetRecordArmParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let id = params.track_id.0;
    let track = match require_external(app, id) {
        Ok(t) => t,
        Err(e) => return reject(request, e),
    };
    if track.record_armed == params.armed {
        return (ack(app, request), Task::none());
    }
    let task = super::run_via_update(app, Message::ExternalInstrument(Ext::ToggleRecordArm(id)));
    (ack(app, request), task)
}

fn set_playback_source(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetPlaybackSourceParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let id = params.track_id.0;
    if let Err(e) = require_external(app, id) {
        return reject(request, e);
    }
    let source = match params.source {
        PlaybackSource::Live => resonance_common::PlaybackSource::Live,
        PlaybackSource::Recorded => resonance_common::PlaybackSource::Recorded,
        PlaybackSource::Unknown => {
            return reject(
                request,
                RpcError::invalid_params("source must be \"live\" or \"recorded\""),
            )
        }
    };
    let task = super::run_via_update(
        app,
        Message::ExternalInstrument(Ext::SetPlaybackSource(id, source)),
    );
    (ack(app, request), task)
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

/// `external.bounce` — drive the realtime bounce-in-place flow the GUI
/// dialog drives: open it for the track, apply the input selection, then
/// confirm. The capture runs in real time from there (the engine mutes
/// everything else, arms a fresh audio track and records the return
/// until the source's MIDI ends), so this reply means "started", not
/// "finished" — the new track appears in `song.tracks` when it lands.
fn bounce(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: BounceParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let id = params.track_id.0;
    let track = match require_external(app, id) {
        Ok(t) => t,
        Err(e) => return reject(request, e),
    };

    let device = params
        .device
        .clone()
        .or_else(|| track.input_device_name.clone());
    let Some(device) = device else {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "track {id} has no audio return to capture from; set one with external.set_return \
                 or pass an explicit device"
            )),
        );
    };
    if !app.input_devices.iter().any(|d| d.name == device) {
        return reject(
            request,
            RpcError::not_found(format!(
                "no audio input named {device:?}; external.devices lists what is connected"
            )),
        );
    }
    if app.transport.playing {
        return reject(
            request,
            RpcError::invalid_params("stop the transport before bouncing"),
        );
    }
    // Nothing to re-drive the synth with means nothing to capture: the
    // realtime bounce plays the source's MIDI and records what comes
    // back.
    if !app.midi_clips.iter().any(|c| c.track_id == id) {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "track {id} has no MIDI to play to the synth, so a bounce would record silence"
            )),
        );
    }

    let port = params.port.unwrap_or(track.input_port_index);
    let mono = params.mono.unwrap_or(track.mono);

    // The five dialog-driving dispatches are ONE undoable transaction:
    // one revision bump for the call, one edit_undo for the whole
    // gesture. If the flow refuses to open mid-group, the group closes
    // with whatever the opening dispatch did as its single entry —
    // that partial state is exactly what the error reply describes,
    // and one edit_undo takes it back.
    use crate::message::BounceMessage as B;
    app.with_compound_undo(|app| {
        let open = super::run_via_update(app, Message::Track(TrackMessage::BounceInPlace(id)));
        // `BounceInPlace` routes an external track to the picker dialog; if
        // it went anywhere else there is nothing to confirm and driving the
        // rest would be a no-op we'd wrongly report as started.
        if app.bounce_dialog.is_none() {
            return reject(
                request,
                RpcError::unsupported(format!(
                    "track {id} did not open the realtime bounce flow; it may be frozen or already \
                     bouncing"
                )),
            );
        }
        let pick_device = super::run_via_update(
            app,
            Message::Track(TrackMessage::Bounce(B::PickDevice(Some(device)))),
        );
        let set_mono = super::run_via_update(app, Message::Track(TrackMessage::Bounce(B::SetMono(mono))));
        // Port last: PickDevice resets it to 0 and SetMono can snap it.
        let pick_port = super::run_via_update(app, Message::Track(TrackMessage::Bounce(B::PickPort(port))));
        let confirm = super::run_via_update(app, Message::Track(TrackMessage::Bounce(B::Confirm)));

        (
            ack(app, request),
            Task::batch([open, pick_device, set_mono, pick_port, confirm]),
        )
    })
}
