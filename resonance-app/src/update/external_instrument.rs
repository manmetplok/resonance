//! Update handlers for external-instrument tracks (architecture doc #169,
//! epic #39).
//!
//! Each [`ExternalInstrumentMessage`] mutates GUI state optimistically and
//! dispatches the matching `AudioCommand`. The MIDI-out / audio-return /
//! monitor / arm controls reuse the plain-track engine commands
//! (`SetTrackMidiOutput`, `SetTrackInputDevice`, `SetTrackInputPort`,
//! `SetTrackMonitor`, `SetTrackRecordArm`) because device/channel and
//! monitor/arm have exactly one source of truth — the engine-side `Track`.
//! Bank/program, latency and the device re-check use the dedicated
//! external-instrument commands. The undo classifier (`crate::undo`) records
//! the config-changing variants; runtime-only variants (`CheckDevices`,
//! `RescanDevices`, `RevealUserDefinitionsFolder`, `RescanDefinitions`) are
//! skipped.

use iced::Task;
use resonance_audio::types::{AudioCommand, TrackId};

use crate::message::{ExternalInstrumentMessage, Message};
use crate::state::ExternalInstrumentState;
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: ExternalInstrumentMessage) -> Task<Message> {
    use ExternalInstrumentMessage as M;
    match m {
        M::Enable(track_id) => {
            // Only meaningful for an existing track; ignore stray ids.
            if r.registry.tracks.iter().any(|t| t.id == track_id) {
                enable_external_instrument(r, track_id);
            }
        }
        M::Disable(track_id) => {
            if r.external_instruments.remove(&track_id).is_some() {
                let _ = r
                    .engine
                    .send(AudioCommand::ClearExternalInstrument { track_id });
            }
        }
        M::SetMidiOutDevice(track_id, device) => {
            let channel = r.with_track_mut(track_id, |t| {
                t.midi_output_device = device.clone();
                t.midi_output_channel
            });
            if let Some(channel) = channel {
                // A freshly-picked output is assumed online until a re-check
                // proves otherwise — the engine only ever reports *offline*.
                if let Some(state) = r.external_instruments.get_mut(&track_id) {
                    state.midi_out_offline = false;
                }
                let _ = r.engine.send(AudioCommand::SetTrackMidiOutput {
                    track_id,
                    device,
                    channel,
                });
            }
        }
        M::SetMidiOutChannel(track_id, channel) => {
            let device = r.with_track_mut(track_id, |t| {
                t.midi_output_channel = channel;
                t.midi_output_device.clone()
            });
            if let Some(device) = device {
                let _ = r.engine.send(AudioCommand::SetTrackMidiOutput {
                    track_id,
                    device,
                    channel,
                });
            }
        }
        M::SetDevice(track_id, device_id) => {
            // Only meaningful for a track already in external-instrument
            // mode. Resolve the chosen preset's params up front (immutable
            // registry borrow), store the id on the track state, then hand
            // the engine the binding map. An unknown id or `None` clears the
            // engine map (empty params) and the selection.
            if r.external_instruments.contains_key(&track_id) {
                let params = match &device_id {
                    Some(id) => r
                        .device_registry
                        .get(id)
                        .map(|def| def.params.clone())
                        .unwrap_or_default(),
                    None => Vec::new(),
                };
                if let Some(state) = r.external_instruments.get_mut(&track_id) {
                    // Keep the selected id even if it didn't resolve to a
                    // known definition — the picker only offers real ids, and
                    // this makes clearing (`None`) unambiguous.
                    state.device_id = device_id;
                }
                let _ = r
                    .engine
                    .send(AudioCommand::SetTrackDeviceParams { track_id, params });
            }
        }
        M::SetReturnDevice(track_id, device_name) => {
            let updated = r.with_track_mut(track_id, |t| {
                t.input_device_name = device_name.clone();
                t.input_port_index = 0;
            });
            if updated.is_some() {
                // Assume the freshly-picked return is online until re-checked.
                if let Some(state) = r.external_instruments.get_mut(&track_id) {
                    state.return_input_offline = false;
                }
                let _ = r.engine.send(AudioCommand::SetTrackInputDevice {
                    track_id,
                    device_name,
                });
                let _ = r
                    .engine
                    .send(AudioCommand::SetTrackInputPort { track_id, port_index: 0 });
            }
        }
        M::SetReturnPort(track_id, port_index) => {
            let updated = r.with_track_mut(track_id, |t| t.input_port_index = port_index);
            if updated.is_some() {
                let _ = r
                    .engine
                    .send(AudioCommand::SetTrackInputPort { track_id, port_index });
            }
        }
        M::SetBank(track_id, bank) => {
            if let Some(state) = r.external_instruments.get_mut(&track_id) {
                state.bank = bank;
                let program = state.program;
                let _ = r.engine.send(AudioCommand::SetExternalInstrumentPatch {
                    track_id,
                    bank,
                    program,
                });
            }
        }
        M::SetProgram(track_id, program) => {
            if let Some(state) = r.external_instruments.get_mut(&track_id) {
                state.program = program;
                let bank = state.bank;
                let _ = r.engine.send(AudioCommand::SetExternalInstrumentPatch {
                    track_id,
                    bank,
                    program,
                });
            }
        }
        M::SetPatch(track_id, bank, program) => {
            // A named-patch pick sets bank + program in one edit and fires a
            // single Bank Select + Program Change — the same engine path the
            // numeric Bank/Program pickers use (doc #201 §5). The named patch
            // persists implicitly via the resolved bank/program.
            if let Some(state) = r.external_instruments.get_mut(&track_id) {
                state.bank = bank;
                state.program = program;
                let _ = r.engine.send(AudioCommand::SetExternalInstrumentPatch {
                    track_id,
                    bank,
                    program,
                });
            }
        }
        M::SetLatencyOffset(track_id, latency_offset_samples) => {
            if let Some(state) = r.external_instruments.get_mut(&track_id) {
                state.latency_offset_samples = latency_offset_samples;
                let _ = r
                    .engine
                    .send(AudioCommand::SetExternalInstrumentLatencyOffset {
                        track_id,
                        latency_offset_samples,
                    });
            }
        }
        M::ToggleMonitor(track_id) => {
            let enabled = r.with_track_mut(track_id, |t| {
                t.monitor_enabled = !t.monitor_enabled;
                t.monitor_enabled
            });
            if let Some(enabled) = enabled {
                let _ = r
                    .engine
                    .send(AudioCommand::SetTrackMonitor { track_id, enabled });
            }
        }
        M::ToggleRecordArm(track_id) => {
            let default_device = r.default_input_device_name.clone();
            let auto = r.with_track_mut(track_id, |t| {
                t.record_armed = !t.record_armed;
                if t.record_armed && t.input_device_name.is_none() {
                    t.input_device_name = default_device.clone();
                }
                (t.record_armed, t.input_device_name.clone())
            });
            if let Some((armed, device)) = auto {
                if armed && device.is_some() {
                    let _ = r.engine.send(AudioCommand::SetTrackInputDevice {
                        track_id,
                        device_name: device,
                    });
                }
                let _ = r
                    .engine
                    .send(AudioCommand::SetTrackRecordArm { track_id, armed });
            }
        }
        M::CheckDevices(track_id) => {
            // Clear offline optimistically, then re-check: the engine
            // re-asserts the offline event for any endpoint still missing,
            // so a recovered device drops back to online with no extra event.
            if let Some(state) = r.external_instruments.get_mut(&track_id) {
                state.midi_out_offline = false;
                state.return_input_offline = false;
                let _ = r
                    .engine
                    .send(AudioCommand::CheckExternalInstrumentDevices { track_id });
            }
        }
        M::RescanDevices => {
            let _ = r.engine.send(AudioCommand::ListInputDevices);
            let _ = r.engine.send(AudioCommand::ListMidiOutputDevices);
            let _ = r.engine.send(AudioCommand::ListMidiInputDevices);
        }
        M::RevealUserDefinitionsFolder => {
            // Ensure the folder exists before opening it so the file manager
            // doesn't error on a directory that has never been created.
            if let Some(dir) = resonance_common::user_definitions_dir() {
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    eprintln!(
                        "RevealUserDefinitionsFolder: could not create {}: {e}",
                        dir.display()
                    );
                    return Task::none();
                }
                reveal_path_in_file_manager(&dir);
            }
        }
        M::RescanDefinitions => {
            // Rebuild the registry from bundled + user folder (last-wins by
            // id, matching startup behaviour in `lib.rs`). Then refresh the
            // cached pick-list options so the device-preset picker shows the
            // new entries on the very next frame.
            let mut registry = resonance_common::DeviceDefinitionRegistry::default();
            registry.scan_bundled();
            if let Some(dir) = resonance_common::user_definitions_dir() {
                registry.scan_dir(&dir);
            }
            r.view_caches
                .rebuild_device_choices(&registry.list());
            r.device_registry = registry;
        }
    }
    Task::none()
}

/// Put `track_id` into external-instrument mode: insert a fresh
/// [`ExternalInstrumentState`] (idempotent — an existing entry is reused) and
/// hand the engine the matching `SetExternalInstrument` config.
///
/// This is the core of `ExternalInstrumentMessage::Enable` minus its
/// "track must already exist" guard, factored out so track creation can enable
/// external mode on a freshly-allocated id in the same undo step
/// (`TrackMessage::AddExternalInstrumentTrack`) without duplicating the logic.
/// Callers that operate on user-supplied ids should keep the existence guard;
/// callers that just allocated the id (and are about to create the track) skip
/// it because the engine echo lands the track a beat later.
pub(crate) fn enable_external_instrument(r: &mut Resonance, track_id: TrackId) {
    let state = r
        .external_instruments
        .entry(track_id)
        .or_insert_with(|| ExternalInstrumentState::new(track_id));
    let config = state.config();
    let _ = r.engine.send(AudioCommand::SetExternalInstrument { config });
}

/// Open `path` in the OS native file manager (Nautilus / Finder / Explorer).
/// Uses platform-specific launchers; failures are logged but not fatal — the
/// user can still navigate there manually.
fn reveal_path_in_file_manager(path: &std::path::Path) {
    #[cfg(target_os = "linux")]
    let cmd = ("xdg-open", &[path.as_os_str()][..]);
    #[cfg(target_os = "macos")]
    let cmd = ("open", &[path.as_os_str()][..]);
    #[cfg(target_os = "windows")]
    let cmd = ("explorer", &[path.as_os_str()][..]);

    if let Err(e) = std::process::Command::new(cmd.0).args(cmd.1).spawn() {
        eprintln!(
            "reveal_path_in_file_manager: failed to open {}: {e}",
            path.display()
        );
    }
}
