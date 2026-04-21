use iced::Task;
use resonance_audio::types::AudioCommand;

use crate::message::{Message, TrackMessage};
use crate::util::db_to_gain;
use crate::Resonance;

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
                r.engine
                    .send(AudioCommand::RemoveTrack { track_id: id });
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
                r.engine
                    .send(AudioCommand::RemoveTrack { track_id: id });
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
            r.engine
                .send(AudioCommand::RemoveTrack { track_id: id });
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
                t.instrument_icon =
                    crate::state::InstrumentIcon::default_for(ty);
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
                r.engine.send(AudioCommand::SetTrackMono {
                    track_id: id,
                    mono,
                });
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
            let updated =
                r.with_track_mut(id, |t| t.input_port_index = port_index);
            if updated.is_some() {
                r.engine.send(AudioCommand::SetTrackInputPort {
                    track_id: id,
                    port_index,
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
    }
    Task::none()
}
