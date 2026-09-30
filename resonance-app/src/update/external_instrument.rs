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
//! `DetectLatency`, `RescanDevices`, `RevealUserDefinitionsFolder`,
//! `RescanDefinitions`) are skipped.

use iced::Task;
use resonance_audio::types::{AudioCommand, TrackId};

use crate::message::Message;
use crate::state::ExternalInstrumentState;
use crate::Resonance;

/// User actions on an external-instrument track's inspector / strip
/// (architecture doc #169, epic #39). Each variant maps to one update
/// handler that mutates GUI state and dispatches the matching
/// `AudioCommand`. The MIDI-out / audio-return / monitor / arm controls
/// reuse the plain-track engine commands (the engine keeps one source of
/// truth for them); the bank/program, latency and device-check controls
/// use the external-instrument commands. No view in this todo.
#[derive(Debug, Clone)]
pub enum ExternalInstrumentMessage {
    /// Turn `track` into an external instrument (or re-assert it), storing a
    /// fresh config when none exists. Dispatches `SetExternalInstrument`.
    Enable(TrackId),
    /// Take `track` out of external-instrument mode, dropping its config.
    /// Dispatches `ClearExternalInstrument`.
    Disable(TrackId),
    /// Pick the hardware MIDI output device (`None` disconnects).
    SetMidiOutDevice(TrackId, Option<String>),
    /// Pick the MIDI output channel (`None` = channel 1).
    SetMidiOutChannel(TrackId, Option<u8>),
    /// Pick a device preset by id (a `DeviceDefinition::id`), or `None` to
    /// clear the selection. Stores the id on the track's external-instrument
    /// state and dispatches `SetTrackDeviceParams` with the definition's
    /// params (empty on clear / unknown id). Epic #40, doc #201 §5.
    SetDevice(TrackId, Option<String>),
    /// Pick the audio-return input device (`None` clears).
    SetReturnDevice(TrackId, Option<String>),
    /// Pick the 0-indexed starting audio-return input port.
    SetReturnPort(TrackId, u16),
    /// Set the selected MIDI bank (combined 14-bit MSB<<7|LSB), or `None` to
    /// send no Bank Select. Fires the patch send.
    SetBank(TrackId, Option<u16>),
    /// Set the selected MIDI program (`0..=127`), or `None` to send no
    /// Program Change. Fires the patch send.
    SetProgram(TrackId, Option<u8>),
    /// Select a **named patch** from the selected device definition (epic
    /// #40, doc #201 §5): sets the combined 14-bit bank (`MSB<<7|LSB`) and
    /// the program together, resolved from the chosen `PatchEntry`. `None`
    /// bank/program clears the corresponding selection (the "(no patch)"
    /// entry sends both `None`). Fires a single Bank Select + Program Change
    /// through the same path as `SetBank`/`SetProgram`.
    SetPatch(TrackId, Option<u16>, Option<u8>),
    /// Set the manual latency offset (samples) aligning the audio return.
    SetLatencyOffset(TrackId, i64),
    /// Toggle input monitoring for the return.
    ToggleMonitor(TrackId),
    /// Toggle record-arm (capture the audio return to the timeline).
    ToggleRecordArm(TrackId),
    /// Pick what the track plays back: `Live` re-drives the hardware from
    /// timeline MIDI, `Recorded` plays recorded takes over the spans they
    /// cover (doc #257). Engine-owned like monitor/arm — dispatches
    /// `SetTrackPlaybackSource`; the engine echoes
    /// `TrackPlaybackSourceChanged`. Auto-switched to `Recorded` when a
    /// take finishes recording on an external-instrument track.
    SetPlaybackSource(TrackId, resonance_common::PlaybackSource),
    /// Auto-detect ping: re-check this track's MIDI-out + audio-return
    /// devices against the live hardware and report any that are offline.
    CheckDevices(TrackId),
    /// Auto-detect the round-trip latency of this external-instrument track:
    /// dispatch `DetectExternalInstrumentLatency` so the engine fires a MIDI
    /// impulse and times the audio return, then reports back via
    /// `ExternalInstrumentLatencyMeasured` / `…LatencyDetectFailed`. No-op if
    /// the track isn't external, a detect is already running, or the transport
    /// is playing (the engine requires a stopped transport). Runtime-only —
    /// the measured offset arrives as a separate engine event; no undo entry.
    DetectLatency(TrackId),
    /// Re-scan the available hardware so the "pick another device" lists are
    /// fresh. Runtime-only — refreshes device lists, mutates no config.
    RescanDevices,
    /// Open the user device-definitions folder in the OS file manager so the
    /// user can add or edit `.json` definition files. Creates the folder
    /// first so the file manager opens something rather than erroring.
    /// Runtime-only — no undo, no config mutation.
    RevealUserDefinitionsFolder,
    /// Re-scan the device-definition registry (bundled + user folder) and
    /// rebuild the device-preset picker options. Call after the user has
    /// dropped a new `.json` file into the user definitions folder.
    /// Runtime-only — no undo.
    RescanDefinitions,
}

impl ExternalInstrumentMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::{CoalesceKey, UndoAction};
        match self {
            // Runtime-only: re-checking devices / re-scanning hardware /
            // revealing the user definitions folder / re-scanning definitions /
            // auto-detecting latency all mutate no project state. The measured
            // offset a detect eventually produces arrives as a separate engine
            // event (mirrored into runtime-only state), not this message.
            Self::CheckDevices(_)
            | Self::RescanDevices
            | Self::RevealUserDefinitionsFolder
            | Self::RescanDefinitions
            | Self::DetectLatency(_) => UndoAction::Skip,
            // The latency offset slider delivers one message per step with no
            // begin/commit pair; coalesce per track so dragging the same track's
            // offset back and forth merges into one entry (FU-A10c).
            Self::SetLatencyOffset(track_id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::ExternalLatency(*track_id))
            }
            // Every other config change (enable/disable, route, patch,
            // monitor, arm, playback source) is a user-meaningful,
            // reversible edit. The playback-source *auto-switch* after a
            // recorded take is event-driven (`RecordingFinished`), not a
            // message, so it never lands an undo entry of its own — only
            // the explicit inspector toggle does.
            Self::Enable(..)
            | Self::Disable(..)
            | Self::SetMidiOutDevice(..)
            | Self::SetMidiOutChannel(..)
            | Self::SetDevice(..)
            | Self::SetReturnDevice(..)
            | Self::SetReturnPort(..)
            | Self::SetBank(..)
            | Self::SetProgram(..)
            | Self::SetPatch(..)
            | Self::ToggleMonitor(..)
            | Self::ToggleRecordArm(..)
            | Self::SetPlaybackSource(..) => UndoAction::Record,
        }
    }
}

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
            if r.devices.external_instruments.remove(&track_id).is_some() {
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
                if let Some(state) = r.devices.external_instruments.get_mut(&track_id) {
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
            if r.devices.external_instruments.contains_key(&track_id) {
                let params = match &device_id {
                    Some(id) => r
                        .devices
                        .registry
                        .get(id)
                        .map(|def| def.params.clone())
                        .unwrap_or_default(),
                    None => Vec::new(),
                };
                if let Some(state) = r.devices.external_instruments.get_mut(&track_id) {
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
                if let Some(state) = r.devices.external_instruments.get_mut(&track_id) {
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
            if let Some(state) = r.devices.external_instruments.get_mut(&track_id) {
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
            if let Some(state) = r.devices.external_instruments.get_mut(&track_id) {
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
            if let Some(state) = r.devices.external_instruments.get_mut(&track_id) {
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
            if let Some(state) = r.devices.external_instruments.get_mut(&track_id) {
                state.latency_offset_samples = latency_offset_samples;
                let _ = r
                    .engine
                    .send(AudioCommand::SetExternalInstrumentLatencyOffset {
                        track_id,
                        latency_offset_samples,
                    });
            }
        }
        M::SetPlaybackSource(track_id, source) => {
            // Engine-owned like monitor/arm: mutate the mirror
            // optimistically and dispatch; the engine echoes
            // `TrackPlaybackSourceChanged`, which re-asserts the same
            // value (idempotent).
            let applied = r.with_track_mut(track_id, |t| {
                t.playback_source = source;
            });
            if applied.is_some() {
                let _ = r
                    .engine
                    .send(AudioCommand::SetTrackPlaybackSource { track_id, source });
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
            let default_device = r.devices.input.default_name.clone();
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
            if let Some(state) = r.devices.external_instruments.get_mut(&track_id) {
                state.midi_out_offline = false;
                state.return_input_offline = false;
                let _ = r
                    .engine
                    .send(AudioCommand::CheckExternalInstrumentDevices { track_id });
            }
        }
        M::DetectLatency(track_id) => {
            // Auto-detect ("ping") the round-trip latency. No-op unless the
            // track is external, no detect is already running, and the
            // transport is stopped (the engine rejects a ping mid-playback).
            let can_detect = !r.transport.playing
                && r.devices.external_instruments
                    .get(&track_id)
                    .is_some_and(|state| !state.latency_detect_in_progress);
            if can_detect {
                if let Some(state) = r.devices.external_instruments.get_mut(&track_id) {
                    state.latency_detect_in_progress = true;
                    // A fresh attempt supersedes any stale failure reason.
                    state.latency_detect_error = None;
                }
                let _ = r
                    .engine
                    .send(AudioCommand::DetectExternalInstrumentLatency { track_id });
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
                    tracing::warn!(
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
            r.ui.view_caches
                .rebuild_device_choices(&registry.list());
            r.devices.registry = registry;
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
    remove_in_app_instrument(r, track_id);
    let state = r
        .devices
        .external_instruments
        .entry(track_id)
        .or_insert_with(|| ExternalInstrumentState::new(track_id));
    let config = state.config();
    let _ = r.engine.send(AudioCommand::SetExternalInstrument { config });
}

/// Drop an in-app instrument left at slot 0 when a track goes external.
///
/// An external track's audio arrives on its RETURN input and every plugin
/// in its chain is an insert, so a leftover synth is not merely unused —
/// it runs as the first insert and OVERWRITES the return signal with its
/// own (noteless, silent) output. Live monitoring survives on the monitor
/// path, which is what made this so expensive to diagnose: the take was
/// audible in the app and every offline render came out at -120 dBFS.
/// Recovery was GUI-only, because `track.remove_effect` refuses slot 0 as
/// structural (ba doc #275 P1.2).
///
/// Removing it is the honest reading of the request: "this track's
/// instrument is outboard" and "this track's instrument is a wavetable"
/// cannot both hold. The removal goes through the same engine command the
/// GUI uses, so it lands in the undo history with the enable.
fn remove_in_app_instrument(r: &mut Resonance, track_id: TrackId) {
    let Some(track) = r.registry.tracks.iter().find(|t| t.id == track_id) else {
        return;
    };
    // Only slot 0, and only when the scanner classifies it as an
    // instrument: an external track may legitimately carry effects, and a
    // chain that starts with an EQ must survive untouched.
    let Some(first) = track.plugins.first() else {
        return;
    };
    let is_instrument = r
        .plugin_catalog
        .available_plugins
        .iter()
        .find(|p| p.clap_plugin_id == first.clap_plugin_id)
        .is_some_and(|p| p.is_instrument);
    if !is_instrument {
        return;
    }
    let instance_id = first.instance_id;
    let _ = r.engine.send(AudioCommand::RemovePlugin {
        track_id,
        instance_id,
    });
}

/// Open `path` in the OS native file manager (Nautilus / Finder / Explorer)
/// through the launcher the app shares with the plugins
/// (`resonance_common::reveal`). Failures are logged but not fatal — the
/// user can still navigate there manually. `pub(crate)` so the freeze slice
/// ("Reveal freeze cache…", ba todo #581) can reuse the launcher.
pub(crate) fn reveal_path_in_file_manager(path: &std::path::Path) {
    if let Err(e) = resonance_common::reveal::reveal(path) {
        tracing::warn!(
            "reveal_path_in_file_manager: failed to open {}: {e}",
            path.display()
        );
    }
}
