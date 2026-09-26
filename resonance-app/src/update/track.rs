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
            let _ = r.engine.send(AudioCommand::AddTrack {
                id_hint: None,
                name: None,
            });
            r.mixer.add_track_menu_open = false;
        }
        TrackMessage::AddInstrumentTrack => {
            let _ = r.engine.send(AudioCommand::AddInstrumentTrack {
                id_hint: None,
                name: None,
            });
            r.mixer.add_track_menu_open = false;
        }
        TrackMessage::AddExternalInstrumentTrack => {
            // Same track creation as `AddInstrumentTrack`, but the id is
            // allocated app-side (like the audio-drop new-track path) so we can
            // immediately enable external mode on it. The engine echoes
            // `InstrumentTrackAdded` for this id a beat later, which mirrors the
            // track into the registry. Enabling external mode here — before the
            // echo — is safe: `enable_external_instrument` only touches
            // `r.external_instruments` + the engine, not the registry, and the
            // engine applies `SetExternalInstrument` after the track exists.
            //
            // Both effects (track creation + external state) fall under one undo
            // snapshot: this message classifies as `UndoAction::Record`, whose
            // pre-dispatch snapshot has neither the track nor the external entry,
            // so a single undo removes both and redo restores both.
            let track_id = r.allocate_track_id();
            let _ = r.engine.send(AudioCommand::AddInstrumentTrack {
                id_hint: Some(track_id),
                name: None,
            });
            crate::update::external_instrument::enable_external_instrument(r, track_id);
            r.mixer.add_track_menu_open = false;
        }
        TrackMessage::AddVocalTrack => {
            let _ = r.engine.send(AudioCommand::AddVocalTrack {
                id_hint: None,
                name: None,
            });
            r.mixer.add_track_menu_open = false;
        }
        TrackMessage::AddControlTrack { id, kind, name } => {
            use crate::state::ControlTrackKind;
            // Id-hinted add so the control reply can return `id`
            // immediately; the engine echoes `*TrackAdded { id }` which
            // mirrors the track into the registry. Drums queue a
            // deferred instrument-type set for that echo.
            let cmd = match kind {
                ControlTrackKind::Vocal => AudioCommand::AddVocalTrack {
                    id_hint: Some(id),
                    name: name.clone(),
                },
                ControlTrackKind::Audio => AudioCommand::AddTrack {
                    id_hint: Some(id),
                    name: name.clone(),
                },
                ControlTrackKind::Instrument
                | ControlTrackKind::Drums
                | ControlTrackKind::External => AudioCommand::AddInstrumentTrack {
                    id_hint: Some(id),
                    name: name.clone(),
                },
            };
            let _ = r.engine.send(cmd);
            // Defer name + drum-type application to the engine echo (the
            // registry mirror ignores the engine's name and always makes
            // a synth track); apply now too, in case the echo already
            // landed (tests drive it synchronously).
            r.control.pending_tracks.insert(
                id,
                crate::state::PendingControlTrack {
                    kind,
                    name: name.clone(),
                },
            );
            r.apply_pending_control_track(id);
            // External mode goes on in the same undo step as the track
            // itself, exactly as `AddExternalInstrumentTrack` does for
            // the Add-Track menu — one undo removes both.
            if matches!(kind, ControlTrackKind::External) {
                crate::update::external_instrument::enable_external_instrument(r, id);
            }
        }
        TrackMessage::RequestRemoveTrack(id) => {
            let has_audio = r.clips.iter().any(|c| c.track_id == id);
            let has_midi = r.midi_clips.iter().any(|c| c.track_id == id);
            // The request itself is `Skip` for undo — opening the confirm
            // dialog is no edit (code review STATE-13). An empty track
            // needs no confirm, so it goes straight to the confirmed
            // delete, recorded exactly as a dispatched `ConfirmRemoveTrack`
            // would be — that delete is the one undo entry.
            r.confirm_delete_track = Some(id);
            if !(has_audio || has_midi) {
                let _ = r.record_undo(&Message::Track(TrackMessage::ConfirmRemoveTrack));
                return handle(r, TrackMessage::ConfirmRemoveTrack);
            }
        }
        TrackMessage::ConfirmRemoveTrack => {
            if let Some(id) = r.confirm_delete_track.take() {
                r.interaction.deselect_track(id);
                if r.compose.expanded_track_id == Some(id) {
                    r.compose.expanded_track_id = None;
                }
                // Tear down any freeze cache the track owned (ba todo #577).
                r.cleanup_freeze_on_delete(id);
                let _ = r.engine.send(AudioCommand::RemoveTrack { track_id: id });
                // Mirror the removal now, not on the `TrackRemoved` echo,
                // so an undo before the echo sees it (code review
                // STATE-10). The echo's handler is idempotent.
                crate::engine_events::tracks::removed(r, id);
            }
        }
        TrackMessage::CancelRemoveTrack => {
            r.confirm_delete_track = None;
        }
        TrackMessage::SetTrackVolume(id, vol_db) => {
            let _ = r.engine.send(AudioCommand::SetTrackVolume {
                track_id: id,
                volume: db_to_gain(vol_db),
            });
            r.with_track_mut(id, |t| t.volume = vol_db);
        }
        TrackMessage::SetTrackPan(id, pan) => {
            let _ = r.engine
                .send(AudioCommand::SetTrackPan { track_id: id, pan });
            r.with_track_mut(id, |t| t.pan = pan);
        }
        TrackMessage::SetMasterVolume(vol_db) => {
            let _ = r.engine.send(AudioCommand::SetMasterVolume {
                volume: db_to_gain(vol_db),
            });
            r.master_volume = vol_db;
        }
        TrackMessage::ToggleMute(id) => {
            let new_muted = r.with_track_mut(id, |t| {
                t.muted = !t.muted;
                t.muted
            });
            if let Some(own) = new_muted {
                // A group's macro mute composes with the track's own mute,
                // so the engine always receives the *effective* mute: the
                // track stays muted while its group mute holds even after
                // its own mute is cleared (todo #687).
                let muted = own || r.track_groups.is_track_muted_via_group(id);
                let _ = r.engine.send(AudioCommand::SetTrackMute {
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
            if let Some(own) = new_soloed {
                // A group's macro solo composes with the track's own solo,
                // so the engine always receives the *effective* solo: the
                // track stays soloed while its group solo holds even after
                // its own solo is cleared (todo #688).
                let soloed = own || r.track_groups.is_track_soloed_via_group(id);
                let _ = r.engine.send(AudioCommand::SetTrackSolo {
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
                    let _ = r.engine.send(AudioCommand::SetTrackInputDevice {
                        track_id: id,
                        device_name: device,
                    });
                }
                let _ = r.engine.send(AudioCommand::SetTrackRecordArm {
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
                let _ = r.engine.send(AudioCommand::SetTrackMonitor {
                    track_id: id,
                    enabled,
                });
            }
        }
        TrackMessage::SetTrackName(track_id, name) => {
            r.with_track_mut(track_id, |t| t.name = name);
        }
        TrackMessage::ToggleTrackFxBypass(id) => {
            let new_bypass = r.with_track_mut(id, |t| {
                t.fx_bypassed = !t.fx_bypassed;
                t.fx_bypassed
            });
            if let Some(bypassed) = new_bypass {
                let _ = r.engine.send(AudioCommand::SetTrackFxBypass {
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
                let _ = r.engine
                    .send(AudioCommand::SetTrackMono { track_id: id, mono });
            }
        }
        TrackMessage::SetTrackInputDevice(id, device_name) => {
            let updated = r.with_track_mut(id, |t| {
                t.input_device_name = device_name.clone();
                t.input_port_index = 0;
            });
            if updated.is_some() {
                let _ = r.engine.send(AudioCommand::SetTrackInputDevice {
                    track_id: id,
                    device_name,
                });
                let _ = r.engine.send(AudioCommand::SetTrackInputPort {
                    track_id: id,
                    port_index: 0,
                });
            }
        }
        TrackMessage::SetTrackInputPort(id, port_index) => {
            let updated = r.with_track_mut(id, |t| t.input_port_index = port_index);
            if updated.is_some() {
                let _ = r.engine.send(AudioCommand::SetTrackInputPort {
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
                let _ = r.engine.send(AudioCommand::SetTrackMidiInput {
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
                let _ = r.engine.send(AudioCommand::SetTrackMidiOutput {
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
                let _ = r.engine.send(AudioCommand::SetTrackMidiInput {
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
                let _ = r.engine.send(AudioCommand::SetTrackMidiOutput {
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
            let _ = r.engine
                .send(AudioCommand::SetTrackOutput { track_id, output });
            r.with_track_mut(track_id, |t| t.output = output);
        }
        TrackMessage::AddTrackFromPreset {
            preset,
            id_hint,
            name,
        } => {
            let track_name = Some(name.unwrap_or_else(|| preset.name.clone()));
            let cmd = match preset.track_type.as_str() {
                "instrument" => AudioCommand::AddInstrumentTrack {
                    id_hint,
                    name: track_name,
                },
                "vocal" => AudioCommand::AddVocalTrack {
                    id_hint,
                    name: track_name,
                },
                _ => AudioCommand::AddTrack {
                    id_hint,
                    name: track_name,
                },
            };
            let _ = r.engine.send(cmd);
            r.pending_track_preset = Some(*preset);
            r.mixer.add_track_menu_open = false;
        }
        TrackMessage::OpenSavePresetPrompt(track_id) => {
            // Seed with the track's own name: it is right often enough
            // to be worth a single Enter, and wrong in a way the user
            // can see before committing.
            let name = r
                .registry
                .tracks
                .iter()
                .find(|t| t.id == track_id)
                .map(|t| t.name.clone())
                .unwrap_or_default();
            let exists = crate::presets::user_preset_exists(&name);
            r.interaction.preset_save = Some(crate::state::PresetSaveState {
                track_id,
                name,
                exists,
            });
            r.interaction.track_menu = None;
        }
        TrackMessage::SetSavePresetName(name) => {
            if let Some(prompt) = r.interaction.preset_save.as_mut() {
                prompt.exists = crate::presets::user_preset_exists(name.trim());
                prompt.name = name;
            }
        }
        TrackMessage::CloseSavePresetPrompt => {
            r.interaction.preset_save = None;
        }
        TrackMessage::SaveTrackAsPreset {
            track_id,
            name,
            overwrite,
        } => {
            handle_save_track_as_preset(r, track_id, name, overwrite);
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
            let _ = r.engine.send(AudioCommand::CancelBounce);
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
    // A realtime bounce plays the project live while an offline control
    // measurement is rendering through the same plugin instances — the
    // same conflict the offline renderers have (see
    // `offline_measure_in_progress`), so it refuses too.
    if r.offline_measure_in_progress() {
        r.error_message = Some("A measurement is in progress; bounce again when it finishes".into());
        r.bounce_dialog = Some(dialog);
        return;
    }

    let source_name = source.name.clone();
    let target_track_id = r.allocate_track_id();
    let track_name = format!("{source_name} bounce");

    let _ = r.engine.send(AudioCommand::AddTrack {
        id_hint: Some(target_track_id),
        name: Some(track_name),
    });
    let _ = r.engine.send(AudioCommand::BounceTrackRealtimeToAudio {
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
/// Arm a track-preset capture (ba todo #1303, finding P1).
///
/// Everything a preset needs except the plugins' opaque state blobs is
/// already in the app; only the engine can ask a plugin for one. So this
/// stores the intent and sends `SaveAllPluginStates`, and the echo
/// (`engine_events::project_io::all_plugin_states_saved`) writes the
/// file. `pending_preset_save` has existed — declared, initialised to
/// `None`, taken on that echo — with nothing in the app ever setting it,
/// which is why the preset menu could only list presets someone had
/// written by hand.
///
/// Refuses rather than replaces: a name already on disk needs
/// `overwrite`, the same rule `track.delete` and the render targets
/// follow. Both surfaces come through here, so the refusal cannot differ
/// between them.
fn handle_save_track_as_preset(
    r: &mut Resonance,
    track_id: resonance_audio::types::TrackId,
    name: String,
    overwrite: bool,
) {
    let name = name.trim().to_string();
    if name.is_empty() {
        r.error_message = Some("Save preset: name a preset before saving it".to_string());
        return;
    }
    if !r.registry.tracks.iter().any(|t| t.id == track_id) {
        r.error_message = Some(format!("Save preset: no track {track_id}"));
        return;
    }
    if !overwrite && crate::presets::user_preset_exists(&name) {
        // The GUI reaches this only if the prompt's own guard was
        // bypassed; it normally offers "Overwrite" instead.
        r.error_message = Some(format!(
            "Save preset: a preset named {name:?} already exists — save it under another name,              or overwrite it"
        ));
        return;
    }

    r.pending_preset_save = Some(crate::PendingPresetSave { track_id, name });
    let _ = r.engine.send(AudioCommand::SaveAllPluginStates);
    r.interaction.preset_save = None;
}

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
    // An offline control measurement holds the offline renderer
    // exclusively; a bounce on top of it would drive the same live
    // plugin instances from two renderers at once (mirrors
    // `meter.measure` refusing while a bounce runs).
    if r.offline_measure_in_progress() {
        r.error_message = Some("A measurement is in progress; bounce again when it finishes".into());
        return;
    }

    match mode {
        BounceMode::External => {
            r.bounce_dialog = Some(crate::state::BounceDialogState {
                source_track_id: track_id,
                selected_device: r.default_input_device_name.clone(),
                selected_port: 0,
                mono: false,
            });
            // Make sure the input device list is fresh for the dialog.
            let _ = r.engine.send(AudioCommand::ListInputDevices);
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
    let target_track_id = r.allocate_track_id();
    let target_clip_id = r.compose.fresh_derived_clip_id();

    let track_name = format!("{source_name} bounce");
    let clip_name = track_name.clone();

    r.bounce_in_progress = Some(crate::state::BounceProgressState {
        mode: crate::state::BounceMode::Offline,
        source_name: source_name.clone(),
        fraction: 0.0,
    });

    let _ = r.engine.send(AudioCommand::AddTrack {
        id_hint: Some(target_track_id),
        name: Some(track_name),
    });
    let _ = r.engine.send(AudioCommand::BounceTrackToAudio {
        source_track_id: track_id,
        target_track_id,
        target_clip_id,
        name: clip_name,
    });
}
