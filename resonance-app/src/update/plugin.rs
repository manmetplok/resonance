use iced::Task;
use resonance_audio::types::AudioCommand;

use crate::message::{Message, PluginMessage};
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: PluginMessage) -> Task<Message> {
    match m {
        PluginMessage::PresetUi(m) => return crate::update::plugin_preset_ui::handle(r, m),
        PluginMessage::SetPluginBypass {
            instance_id,
            bypassed,
        } => {
            // No optimistic mirror: the engine crossfades and its
            // `PluginBypassChanged` echo is what moves `slot.bypassed`.
            // Sending regardless of the mirrored value keeps this a SET —
            // and a slot the app believes is already bypassed but the
            // engine does not (a restore that raced a load) still lands.
            let _ = r
                .engine
                .send(resonance_audio::types::AudioCommand::SetPluginBypass {
                    instance_id,
                    bypassed,
                });
        }
        PluginMessage::AddPluginToTrack(track_id, plugin) => {
            // App-allocated (ARCH-04 D-1), but still fire-and-forget: no
            // placeholder is mirrored, so the mixer shows nothing until
            // the engine's `PluginAdded` echo lands — same as before D-1,
            // only the id's origin changed.
            let id = r.allocate_plugin_id();
            let _ = r.engine.send(AudioCommand::AddPlugin {
                track_id,
                clap_file_path: plugin.clap_file_path,
                clap_plugin_id: plugin.clap_plugin_id,
                id,
            });
        }
        PluginMessage::AddPluginToTrackWithId {
            track_id,
            instance_id,
            plugin,
        } => {
            let _ = r.engine.send(AudioCommand::AddPlugin {
                track_id,
                clap_file_path: plugin.clap_file_path.clone(),
                clap_plugin_id: plugin.clap_plugin_id.clone(),
                id: instance_id,
            });
            // Mirror the slot NOW, with an empty param list, exactly as
            // the project-load replay does (`replay_plugins`). The
            // engine's `PluginAdded` echo finds this slot by
            // `instance_id` and fills in `params`/`has_gui` instead of
            // pushing a second one, so no duplicate appears.
            if let Some(track) = r.registry.tracks.iter_mut().find(|t| t.id == track_id) {
                track.plugins.push(crate::state::PluginSlotState::new(
                    instance_id,
                    plugin.name,
                    plugin.clap_plugin_id,
                    plugin.clap_file_path,
                    Vec::new(),
                    false,
                ));
                r.insert_plugin_index(instance_id, crate::state::PluginLocator::Track(track_id));
            }
        }
        PluginMessage::RemovePluginFromTrack(track_id, instance_id) => {
            // Mirror the removal now, not on the `PluginRemoved` echo, so
            // an undo pressed before the echo lands sees it (STATE-10
            // shape, ARCH-01 FU-A13c). The echo is owed, so a late one
            // cannot drop an instance an undo re-added under this id
            // (A-13h).
            let _ = r.engine.send(AudioCommand::RemovePlugin {
                track_id,
                instance_id,
            });
            r.io.restore_echoes.expect_plugin_removed(instance_id);
            crate::engine_events::plugins::track_removed(r, track_id, instance_id);
        }
        PluginMessage::MovePluginInTrack {
            track_id,
            instance_id,
            to_index,
        } => {
            // The instrument-floor rule is enforced in `gates.rs` before
            // undo/revision bookkeeping runs, so a refused move never
            // reaches here — see `plugin_chain`. What is left to do is
            // apply the end-clamp, which turns "move it last" into a
            // real slot without the caller counting the chain.
            let Some(track) = r.registry.tracks.iter().find(|t| t.id == track_id) else {
                return Task::none();
            };
            let Some(moving) = track
                .plugins
                .iter()
                .position(|p| p.instance_id == instance_id)
            else {
                return Task::none();
            };
            let requested = u32::try_from(to_index).unwrap_or(u32::MAX);
            let to_index =
                match crate::plugin_chain::resolve_effect_move(r, track, moving as u32, requested) {
                    Ok(slot) => slot as usize,
                    Err(_) => return Task::none(),
                };
            let _ = r.engine.send(AudioCommand::MovePlugin {
                track_id,
                instance_id,
                to_index,
            });
            // Mirror it now rather than waiting for `PluginMoved`: the
            // app's `Vec` order is what the mixer draws and what project
            // serialization writes, and a control client must be able to
            // read back the order it just set. `track_moved` replays the
            // same move on the echo and no-ops when it already matches.
            crate::engine_events::plugins::mirror_track_plugin_move(
                r,
                track_id,
                instance_id,
                to_index,
            );
        }
        PluginMessage::ReplacePlugin {
            instance_id,
            plugin,
        } => {
            crate::update::plugin_replace::replace_plugin_slot(r, instance_id, plugin);
        }
        PluginMessage::SetPluginParam(instance_id, param_id, value) => {
            let _ = r.engine.send(AudioCommand::SetPluginParam {
                instance_id,
                param_id,
                value,
            });
            r.with_plugin_mut(instance_id, |p| {
                if let Some(param) = p.params.iter_mut().find(|pp| pp.id == param_id) {
                    param.current_value = value;
                }
            });
            // A plugin that reports its own modified flag compares; for one
            // that does not, a host edit is the one edit the host sees.
            if let Some(identity) = r.presets.plugin_preset_identity.get_mut(&instance_id) {
                if !identity.reported {
                    identity.modified = true;
                }
            }
        }
        m @ PluginMessage::LoadPluginPreset { .. } => apply_preset_load(r, m),
        m @ PluginMessage::LoadPluginPresetFromLocation { .. } => apply_preset_load(r, m),
        PluginMessage::PresetStep { load, .. } => {
            r.presets.debounce_state_load = true;
            apply_preset_load(r, *load);
            r.presets.debounce_state_load = false;
        }
        PluginMessage::SetPluginSidechain {
            instance_id,
            source,
            enabled,
        } => {
            // Mirror optimistically, then command. The engine echoes
            // `SidechainRouteChanged` and the mirror is reconciled to
            // that echo, so the two can only disagree for the length of
            // one event-pump tick — but the mirror has to hold the route
            // *now*, because a save taken before the echo lands would
            // otherwise write a project file with no key in it (ba todo
            // #1311).
            match source {
                Some(source) => {
                    r.sidechain.upsert(resonance_audio::types::SidechainRoute {
                        plugin: instance_id,
                        source,
                        enabled,
                    });
                    let _ = r.engine.send(AudioCommand::SetSidechainRoute {
                        plugin: instance_id,
                        source,
                        enabled,
                    });
                }
                None => {
                    r.sidechain.clear_plugin(instance_id);
                    let _ = r.engine.send(AudioCommand::ClearSidechainRoute {
                        plugin: instance_id,
                    });
                }
            }
        }
        PluginMessage::OpenPluginEditor(instance_id) => {
            // Opening a slot's window focuses it, whichever window it is
            // (mixer-cleanup.md §2.1), and selects its channel: the
            // inspector and the preset commands follow.
            crate::update::plugin_window::focus(r, instance_id);
            // NOT set optimistically (ba todo #1347). The engine reports
            // both outcomes as `AudioEvent::PluginEditorState`, so the
            // mirror is moved by the echo alone. Setting it here made a
            // plugin whose window failed to open read "Close Editor" over
            // nothing, with no way back — the engine said only
            // `Error("Failed to open plugin editor")`, naming no instance.
            let _ = r.engine
                .send(AudioCommand::OpenPluginEditor { instance_id });
        }
        PluginMessage::ClosePluginEditor(instance_id) => {
            // Same rule in the other direction: `handle_close_plugin_editor`
            // emits `closed` unconditionally, including for an unknown
            // instance and for a close that was a no-op, so the echo can
            // always be trusted to clear the flag.
            let _ = r.engine
                .send(AudioCommand::ClosePluginEditor { instance_id });
            let _ = r.engine.send(AudioCommand::SavePluginState { instance_id });
        }
        PluginMessage::RescanPlugins => {
            // Clear the previous run's failures up front: leaving them
            // on screen after a rescan that fixed them would report a
            // problem that no longer exists. The engine refills them
            // (with `PluginScanFailed`) only if this scan hits any.
            r.plugin_catalog.plugin_scan_failures.clear();
            r.plugin_catalog.plugin_scan_in_progress = true;
            let _ = r.engine.send(AudioCommand::RescanPlugins);
        }
        PluginMessage::OpenPluginWindow(instance_id) => {
            return crate::update::plugin_window::open(r, instance_id);
        }
        PluginMessage::ClosePluginWindow(instance_id) => {
            r.ui.mixer.close_plugin_window_for(instance_id);
        }
        PluginMessage::PluginWindowDrag(step) => {
            crate::update::plugin_window::drag(r, step);
        }
        PluginMessage::OpenGenericParams(instance_id) => {
            crate::update::plugin_window::open_generic(r, instance_id);
        }
        PluginMessage::FocusSlot(instance_id) => {
            crate::update::plugin_window::focus(r, instance_id);
        }
        PluginMessage::ChainUi(m) => return crate::update::chain_ui::update(r, m),
    }
    Task::none()
}

/// Apply a [`PluginMessage::LoadPluginPreset`]: the identity, every
/// param (engine and mirror), then the rest of the sound. The message
/// handler records it as one undo entry; an audition and a preset loaded
/// onto a plugin that was just added call this directly, unrecorded
/// (plugin-preset-library.md §6.7). Any other message is ignored.
pub(crate) fn apply_preset_load(r: &mut Resonance, m: PluginMessage) {
    if let PluginMessage::LoadPluginPresetFromLocation {
        instance_id,
        location,
        load_key,
        preset_name,
        preset_id,
    } = m
    {
        // The plugin loads it; its `loaded()` (or its own report) confirms
        // the identity and the engine's refresh moves the mirror.
        let reported = r
            .presets
            .plugin_preset_identity
            .get(&instance_id)
            .is_some_and(|i| i.reported);
        r.presets.plugin_preset_identity.insert(
            instance_id,
            crate::state::presets::SlotPresetIdentity {
                source: resonance_control::methods::plugin_preset::PluginPresetSource::Factory,
                id: preset_id,
                name: preset_name,
                modified: false,
                reported,
            },
        );
        let capture = take_capture(r, instance_id);
        owe_after(r, instance_id, capture);
        let _ = r.engine.send(AudioCommand::LoadPluginPresetFromLocation {
            instance_id,
            location,
            load_key,
            capture,
        });
        return;
    }
    let PluginMessage::LoadPluginPreset {
        instance_id,
        values,
        preset_name,
        preset_state,
        preset_id,
        preset_source,
    } = m
    else {
        return;
    };
    // The identity, optimistically: a reporting plugin confirms it
    // (and keeps `reported`); for any other it is all there is.
    let reported = r
        .presets
        .plugin_preset_identity
        .get(&instance_id)
        .is_some_and(|i| i.reported);
    r.presets.plugin_preset_identity.insert(
        instance_id,
        crate::state::presets::SlotPresetIdentity {
            source: preset_source,
            id: preset_id,
            name: preset_name,
            modified: false,
            reported,
        },
    );
    // Same two steps as SetPluginParam, once per parameter: tell
    // the engine, then move the app's mirror so every reader
    // (generic panel, `track.plugin_params`, its MCP tool) agrees
    // with the sound.
    for (param_id, value) in &values {
        let _ = r.engine.send(AudioCommand::SetPluginParam {
            instance_id,
            param_id: *param_id,
            value: *value,
        });
    }
    r.with_plugin_mut(instance_id, |p| {
        for (param_id, value) in &values {
            if let Some(param) = p.params.iter_mut().find(|pp| pp.id == *param_id) {
                param.current_value = *value;
            }
        }
    });
    // Then the rest of the sound and the identity. After the
    // params in the engine's queue, so a plugin that lays the
    // preset over its state sees the new values.
    if let Some(data) = preset_state {
        let capture = take_capture(r, instance_id);
        if r.presets.debounce_state_load {
            // A run of steps loads only where it stops; the run's first
            // capture is the one its undo entry waits on.
            let entry = r
                .presets
                .pending_step_state
                .entry(instance_id)
                .or_insert_with(|| (Vec::new(), None, std::time::Instant::now()));
            entry.0 = data;
            entry.1 = entry.1.or(capture);
            entry.2 = std::time::Instant::now();
        } else {
            r.presets.pending_step_state.remove(&instance_id);
            owe_after(r, instance_id, capture);
            let _ = r.engine.send(AudioCommand::LoadPluginPresetState {
                instance_id,
                data,
                capture,
            });
        }
    }
}

/// A load with a capture is owed its "after" state (see
/// `PresetState::pending_after`).
fn owe_after(
    r: &mut Resonance,
    instance_id: resonance_audio::types::PluginInstanceId,
    capture: Option<u64>,
) {
    if let Some(token) = capture {
        r.presets.pending_after.insert(
            token,
            crate::state::presets::PendingAfter {
                instance_id,
                ..Default::default()
            },
        );
    }
}

/// How long a run of preset steps waits, after its last step, before the
/// plugin's state load goes out.
pub(crate) const STEP_STATE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(150);

/// Send the state load of every step run that has been quiet for
/// [`STEP_STATE_DEBOUNCE`] (`force` sends them all). Tick.
pub(crate) fn flush_step_state(r: &mut Resonance, force: bool) {
    let due: Vec<_> = r
        .presets
        .pending_step_state
        .iter()
        .filter(|(_, (_, _, at))| force || at.elapsed() >= STEP_STATE_DEBOUNCE)
        .map(|(id, _)| *id)
        .collect();
    for instance_id in due {
        if let Some((data, capture, _)) = r.presets.pending_step_state.remove(&instance_id) {
            owe_after(r, instance_id, capture);
            let _ = r.engine.send(AudioCommand::LoadPluginPresetState {
                instance_id,
                data,
                capture,
            });
        }
    }
}

/// The capture token a preset load onto `instance_id` asks the engine
/// for, if any: an audition's origin (forced), else the late slot of the
/// undo entry this load just recorded. A kept audition's slot takes the
/// origin's state instead, and asks for nothing.
fn take_capture(
    r: &mut Resonance,
    instance_id: resonance_audio::types::PluginInstanceId,
) -> Option<u64> {
    if let Some(token) = r.presets.forced_capture.take() {
        return Some(token);
    }
    let late = r
        .presets
        .next_capture
        .take()
        .filter(|(id, _)| *id == instance_id)
        .map(|(_, late)| late)?;
    if let Some((origin, token)) = r.presets.capture_from.take() {
        let known = origin.lock().ok().and_then(|b| b.clone());
        match known {
            Some(blob) => {
                if let Ok(mut slot) = late.lock() {
                    *slot = Some(blob);
                }
            }
            None => r.presets.pending_captures.entry(token).or_default().push(late),
        }
        return None;
    }
    r.presets.capture_seq += 1;
    let token = r.presets.capture_seq;
    r.presets.pending_captures.entry(token).or_default().push(late);
    Some(token)
}
