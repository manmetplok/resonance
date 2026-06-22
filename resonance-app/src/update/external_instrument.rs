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
//! `RescanDevices`) are skipped.

use iced::Task;
use resonance_audio::types::AudioCommand;

use crate::message::{ExternalInstrumentMessage, Message};
use crate::state::ExternalInstrumentState;
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: ExternalInstrumentMessage) -> Task<Message> {
    use ExternalInstrumentMessage as M;
    match m {
        M::Enable(track_id) => {
            // Only meaningful for an existing track; ignore stray ids.
            if r.registry.tracks.iter().any(|t| t.id == track_id) {
                let state = r
                    .external_instruments
                    .entry(track_id)
                    .or_insert_with(|| ExternalInstrumentState::new(track_id));
                let config = state.config();
                let _ = r.engine.send(AudioCommand::SetExternalInstrument { config });
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
    }
    Task::none()
}
