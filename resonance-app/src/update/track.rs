use iced::Task;
use resonance_audio::types::AudioCommand;

use crate::message::{BounceMessage, Message, TrackMessage};
use crate::state::TrackState;
use crate::util::db_to_gain;
use crate::Resonance;

/// Where a "bounce in place" request should route. Computed from a
/// track and the project's MIDI clip list — the view uses it to grey
/// out the trigger button, and the update layer uses it to dispatch
/// either the offline render or the realtime input-picker dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BounceMode {
    /// Track has at least one synth plugin: render offline.
    Internal,
    /// Track drives external MIDI hardware: open the input picker
    /// dialog so the user picks which audio input to record from.
    External,
}

/// Classify a bounce request. Returns the routing mode on success or a
/// user-facing reason string when the track isn't bounce-able. The view
/// only inspects `is_ok()`; the update layer surfaces the message.
///
/// When a track has both an internal synth and a configured MIDI Out,
/// the external path wins — the user explicitly wired hardware output
/// for a reason and that's the "interesting" sound source.
pub fn classify_bounce(
    track: &TrackState,
    project_midi_clips: impl Iterator<Item = resonance_audio::types::TrackId>,
) -> Result<BounceMode, &'static str> {
    use resonance_audio::types::TrackType;
    if track.track_type != TrackType::Instrument {
        return Err("Bounce in place is only available on instrument tracks");
    }
    if track.sub_track.is_some() {
        return Err(
            "Bounce a parent track to capture its sub-tracks, not a sub-track itself",
        );
    }
    if !project_midi_clips.into_iter().any(|tid| tid == track.id) {
        return Err("Source track has no MIDI clips to bounce");
    }
    let has_external_midi = track.midi_output_device.is_some();
    let has_synth = !track.plugins.is_empty();
    if has_external_midi {
        Ok(BounceMode::External)
    } else if has_synth {
        Ok(BounceMode::Internal)
    } else {
        Err("Bounce: track has no sound source (no internal synth or MIDI Out)")
    }
}

pub fn handle(r: &mut Resonance, m: TrackMessage) -> Task<Message> {
    match m {
        TrackMessage::AddTrack => {
            r.engine.send(AudioCommand::AddTrack {
                id_hint: None,
                name: None,
            });
            r.mixer.add_track_menu_open = false;
        }
        TrackMessage::AddInstrumentTrack => {
            r.engine.send(AudioCommand::AddInstrumentTrack {
                id_hint: None,
                name: None,
            });
            r.mixer.add_track_menu_open = false;
        }
        TrackMessage::RequestRemoveTrack(id) => {
            let has_audio = r.clips.iter().any(|c| c.track_id == id);
            let has_midi = r.midi_clips.iter().any(|c| c.track_id == id);
            if has_audio || has_midi {
                r.confirm_delete_track = Some(id);
            } else {
                if r.interaction.selected_track == Some(id) {
                    r.interaction.selected_track = None;
                }
                if r.compose.expanded_track_id == Some(id) {
                    r.compose.expanded_track_id = None;
                }
                r.engine.send(AudioCommand::RemoveTrack { track_id: id });
            }
        }
        TrackMessage::ConfirmRemoveTrack => {
            if let Some(id) = r.confirm_delete_track.take() {
                if r.interaction.selected_track == Some(id) {
                    r.interaction.selected_track = None;
                }
                if r.compose.expanded_track_id == Some(id) {
                    r.compose.expanded_track_id = None;
                }
                r.engine.send(AudioCommand::RemoveTrack { track_id: id });
            }
        }
        TrackMessage::CancelRemoveTrack => {
            r.confirm_delete_track = None;
        }
        TrackMessage::RemoveTrack(id) => {
            if r.interaction.selected_track == Some(id) {
                r.interaction.selected_track = None;
            }
            if r.compose.expanded_track_id == Some(id) {
                r.compose.expanded_track_id = None;
            }
            r.engine.send(AudioCommand::RemoveTrack { track_id: id });
        }
        TrackMessage::SetTrackVolume(id, vol_db) => {
            r.engine.send(AudioCommand::SetTrackVolume {
                track_id: id,
                volume: db_to_gain(vol_db),
            });
            r.with_track_mut(id, |t| t.volume = vol_db);
        }
        TrackMessage::SetTrackPan(id, pan) => {
            r.engine
                .send(AudioCommand::SetTrackPan { track_id: id, pan });
            r.with_track_mut(id, |t| t.pan = pan);
        }
        TrackMessage::SetMasterVolume(vol_db) => {
            r.engine.send(AudioCommand::SetMasterVolume {
                volume: db_to_gain(vol_db),
            });
            r.master_volume = vol_db;
        }
        TrackMessage::ToggleMute(id) => {
            let new_muted = r.with_track_mut(id, |t| {
                t.muted = !t.muted;
                t.muted
            });
            if let Some(muted) = new_muted {
                r.engine.send(AudioCommand::SetTrackMute {
                    track_id: id,
                    muted,
                });
            }
        }
        TrackMessage::ToggleSolo(id) => {
            let new_soloed = r.with_track_mut(id, |t| {
                t.soloed = !t.soloed;
                t.soloed
            });
            if let Some(soloed) = new_soloed {
                r.engine.send(AudioCommand::SetTrackSolo {
                    track_id: id,
                    soloed,
                });
            }
        }
        TrackMessage::ToggleRecordArm(id) => {
            let default_device = r.default_input_device_name.clone();
            let auto_device = r.with_track_mut(id, |t| {
                t.record_armed = !t.record_armed;
                if t.record_armed && t.input_device_name.is_none() {
                    t.input_device_name = default_device.clone();
                }
                (t.record_armed, t.input_device_name.clone())
            });
            if let Some((armed, device)) = auto_device {
                if armed && device.is_some() {
                    r.engine.send(AudioCommand::SetTrackInputDevice {
                        track_id: id,
                        device_name: device,
                    });
                }
                r.engine.send(AudioCommand::SetTrackRecordArm {
                    track_id: id,
                    armed,
                });
            }
        }
        TrackMessage::ToggleMonitor(id) => {
            let new_enabled = r.with_track_mut(id, |t| {
                t.monitor_enabled = !t.monitor_enabled;
                t.monitor_enabled
            });
            if let Some(enabled) = new_enabled {
                r.engine.send(AudioCommand::SetTrackMonitor {
                    track_id: id,
                    enabled,
                });
            }
        }
        TrackMessage::SetTrackName(track_id, name) => {
            r.with_track_mut(track_id, |t| t.name = name);
        }
        TrackMessage::SetInstrumentType(track_id, ty) => {
            r.with_track_mut(track_id, |t| {
                t.instrument_type = ty;
                t.instrument_icon = crate::state::InstrumentIcon::default_for(ty);
            });
        }
        TrackMessage::SetInstrumentIcon(track_id, icon) => {
            r.with_track_mut(track_id, |t| t.instrument_icon = icon);
        }
        TrackMessage::ToggleTrackFxBypass(id) => {
            let new_bypass = r.with_track_mut(id, |t| {
                t.fx_bypassed = !t.fx_bypassed;
                t.fx_bypassed
            });
            if let Some(bypassed) = new_bypass {
                r.engine.send(AudioCommand::SetTrackFxBypass {
                    track_id: id,
                    bypassed,
                });
            }
        }
        TrackMessage::ToggleTrackMono(id) => {
            let new_mono = r.with_track_mut(id, |t| {
                t.mono = !t.mono;
                t.mono
            });
            if let Some(mono) = new_mono {
                r.engine
                    .send(AudioCommand::SetTrackMono { track_id: id, mono });
            }
        }
        TrackMessage::SetTrackInputDevice(id, device_name) => {
            let updated = r.with_track_mut(id, |t| {
                t.input_device_name = device_name.clone();
                t.input_port_index = 0;
            });
            if updated.is_some() {
                r.engine.send(AudioCommand::SetTrackInputDevice {
                    track_id: id,
                    device_name,
                });
                r.engine.send(AudioCommand::SetTrackInputPort {
                    track_id: id,
                    port_index: 0,
                });
            }
        }
        TrackMessage::SetTrackInputPort(id, port_index) => {
            let updated = r.with_track_mut(id, |t| t.input_port_index = port_index);
            if updated.is_some() {
                r.engine.send(AudioCommand::SetTrackInputPort {
                    track_id: id,
                    port_index,
                });
            }
        }
        TrackMessage::SetTrackMidiInputDevice(id, device) => {
            let updated = r.with_track_mut(id, |t| {
                t.midi_input_device = device.clone();
                t.midi_input_channel
            });
            if let Some(channel) = updated {
                r.engine.send(AudioCommand::SetTrackMidiInput {
                    track_id: id,
                    device,
                    channel,
                });
            }
        }
        TrackMessage::SetTrackMidiOutputDevice(id, device) => {
            let updated = r.with_track_mut(id, |t| {
                t.midi_output_device = device.clone();
                t.midi_output_channel
            });
            if let Some(channel) = updated {
                r.engine.send(AudioCommand::SetTrackMidiOutput {
                    track_id: id,
                    device,
                    channel,
                });
            }
        }
        TrackMessage::SetTrackMidiInputChannel(id, channel) => {
            let device = r.with_track_mut(id, |t| {
                t.midi_input_channel = channel;
                t.midi_input_device.clone()
            });
            if let Some(device) = device {
                r.engine.send(AudioCommand::SetTrackMidiInput {
                    track_id: id,
                    device,
                    channel,
                });
            }
        }
        TrackMessage::SetTrackMidiOutputChannel(id, channel) => {
            let device = r.with_track_mut(id, |t| {
                t.midi_output_channel = channel;
                t.midi_output_device.clone()
            });
            if let Some(device) = device {
                r.engine.send(AudioCommand::SetTrackMidiOutput {
                    track_id: id,
                    device,
                    channel,
                });
            }
        }
        TrackMessage::ToggleSubTracksVisible(id) => {
            if !r.mixer.expanded_sub_track_parents.insert(id) {
                r.mixer.expanded_sub_track_parents.remove(&id);
            }
        }
        TrackMessage::SetTrackOutput(track_id, output) => {
            r.engine
                .send(AudioCommand::SetTrackOutput { track_id, output });
            r.with_track_mut(track_id, |t| t.output = output);
        }
        TrackMessage::AddTrackFromPreset(preset) => {
            r.pending_track_preset = Some(*preset.clone());
            if preset.track_type == "instrument" {
                r.engine.send(AudioCommand::AddInstrumentTrack {
                    id_hint: None,
                    name: Some(preset.name.clone()),
                });
            } else {
                r.engine.send(AudioCommand::AddTrack {
                    id_hint: None,
                    name: Some(preset.name.clone()),
                });
            }
            r.mixer.add_track_menu_open = false;
        }
        TrackMessage::DeleteUserPreset(name) => {
            if let Err(e) = crate::presets::delete_user_preset(&name) {
                r.error_message = Some(format!("Delete preset: {e}"));
            }
            r.user_presets = crate::presets::load_user_presets();
        }
        TrackMessage::BounceInPlace(track_id) => {
            handle_bounce_in_place(r, track_id);
        }
        TrackMessage::Bounce(BounceMessage::PickDevice(device)) => {
            if let Some(d) = r.bounce_dialog.as_mut() {
                d.selected_device = device;
                d.selected_port = 0;
            }
        }
        TrackMessage::Bounce(BounceMessage::PickPort(port)) => {
            if let Some(d) = r.bounce_dialog.as_mut() {
                d.selected_port = port;
            }
        }
        TrackMessage::Bounce(BounceMessage::SetMono(mono)) => {
            if let Some(d) = r.bounce_dialog.as_mut() {
                d.mono = mono;
                // Stereo pairs need an even start channel; switching back
                // to stereo from a port that became invalid would dump the
                // user on the right channel of an old pair. Snap to 0.
                if !mono && d.selected_port % 2 != 0 {
                    d.selected_port = 0;
                }
            }
        }
        TrackMessage::Bounce(BounceMessage::Cancel) => {
            r.bounce_dialog = None;
        }
        TrackMessage::Bounce(BounceMessage::CancelInProgress) => {
            // Engine clears `bounce_in_progress` when it emits
            // `TrackBounceCancelled`; don't drop it locally so the
            // modal stays up while the engine teardown runs (offline
            // is fast; realtime needs the audio thread to settle).
            r.engine.send(AudioCommand::CancelBounce);
        }
        TrackMessage::Bounce(BounceMessage::Confirm) => {
            handle_bounce_dialog_confirm(r);
        }
    }
    Task::none()
}

fn handle_bounce_dialog_confirm(r: &mut Resonance) {
    let Some(dialog) = r.bounce_dialog.take() else {
        return;
    };
    let Some(device) = dialog.selected_device.clone() else {
        r.error_message = Some("Pick an audio input device first".into());
        // Keep the dialog open by re-stashing it.
        r.bounce_dialog = Some(dialog);
        return;
    };
    let Some(source) = r.registry.tracks.iter().find(|t| t.id == dialog.source_track_id) else {
        r.error_message = Some("Bounce: source track not found".into());
        return;
    };
    if r.transport.playing {
        r.error_message = Some("Stop transport before bouncing".into());
        r.bounce_dialog = Some(dialog);
        return;
    }

    let source_name = source.name.clone();
    let target_track_id = r.registry.next_sub_track_id;
    r.registry.next_sub_track_id += 1;
    let track_name = format!("{source_name} bounce");

    r.engine.send(AudioCommand::AddTrack {
        id_hint: Some(target_track_id),
        name: Some(track_name),
    });
    r.engine.send(AudioCommand::BounceTrackRealtimeToAudio {
        source_track_id: dialog.source_track_id,
        target_track_id,
        input_device_name: device,
        input_port_index: dialog.selected_port,
        mono: dialog.mono,
    });
    r.bounce_in_progress = Some(crate::state::BounceProgressState {
        mode: crate::state::BounceMode::Realtime,
        source_name,
        fraction: 0.0,
    });
}

/// Dispatch a "bounce in place" request — runs the source-track
/// classifier and either fires the offline render command (internal
/// synth) or opens the realtime input-picker dialog (external MIDI).
fn handle_bounce_in_place(r: &mut Resonance, track_id: resonance_audio::types::TrackId) {
    let Some(source) = r.registry.tracks.iter().find(|t| t.id == track_id) else {
        r.error_message = Some("Bounce: source track not found".into());
        return;
    };
    let mode = match classify_bounce(source, r.midi_clips.iter().map(|c| c.track_id)) {
        Ok(mode) => mode,
        Err(msg) => {
            r.error_message = Some(msg.into());
            return;
        }
    };
    if r.transport.playing {
        r.error_message = Some("Stop transport before bouncing".into());
        return;
    }

    match mode {
        BounceMode::External => {
            r.bounce_dialog = Some(crate::view::bounce_dialog::BounceDialogState {
                source_track_id: track_id,
                selected_device: r.default_input_device_name.clone(),
                selected_port: 0,
                mono: false,
            });
            // Make sure the input device list is fresh for the dialog.
            r.engine.send(AudioCommand::ListInputDevices);
        }
        BounceMode::Internal => {
            internal_bounce_dispatch(r, track_id);
        }
    }
}

/// Allocate the target track + clip ids and fire the offline bounce
/// command. Caller has already validated the source track.
fn internal_bounce_dispatch(r: &mut Resonance, track_id: resonance_audio::types::TrackId) {
    let source_name = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .map(|t| t.name.clone())
        .unwrap_or_default();
    let target_track_id = r.registry.next_sub_track_id;
    r.registry.next_sub_track_id += 1;
    let target_clip_id = r.compose.fresh_derived_clip_id();

    let track_name = format!("{source_name} bounce");
    let clip_name = track_name.clone();

    r.bounce_in_progress = Some(crate::state::BounceProgressState {
        mode: crate::state::BounceMode::Offline,
        source_name: source_name.clone(),
        fraction: 0.0,
    });

    r.engine.send(AudioCommand::AddTrack {
        id_hint: Some(target_track_id),
        name: Some(track_name),
    });
    r.engine.send(AudioCommand::BounceTrackToAudio {
        source_track_id: track_id,
        target_track_id,
        target_clip_id,
        name: clip_name,
    });
}
