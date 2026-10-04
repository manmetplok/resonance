//! MIDI Learn and hardware controller maps (doc #167, epic #21): arming
//! learn, the right-click MIDI menu, editing the project's bindings, the
//! control-surface port, the controller-map presets — and applying a
//! hardware move.
//!
//! **Bindings are project state.** They persist in `project.json`
//! (`midi_bindings`) and every edit is one undo entry. Each edit updates
//! [`MidiMapState`](crate::state::MidiMapState) at once and sends the
//! engine the matching command; the engine's echoes re-apply it.
//!
//! **A hardware move applies through the on-screen control's own
//! message.** The engine reports `ControlSurfaceMoved { binding, value }`
//! and [`move_message`] turns it into `SetTrackVolume`, `SetPluginParam`,
//! `ToggleMute`, `Play`, … — so a fader on a controller coalesces into
//! one undo entry exactly like a mouse drag on the same fader, marks the
//! project dirty, and writes the model the view draws. Soft takeover and
//! relative encoders read the target's current value here, where it is.

use iced::Task;
use resonance_audio::types::AudioCommand;
use resonance_common::{
    apply_delta, cc_to_norm, decode_relative, lane_value_to_plugin_param, lane_value_to_real,
    real_to_lane_value, takeover_value, AutomationTarget, BindingId, CcMode, ControlSource,
    ControllerMap, MidiBinding, MidiTarget, TransportAction,
};

use crate::message::*;
use crate::state::MidiMenuState;
use crate::undo::UndoAction;
use crate::Resonance;

#[derive(Debug, Clone)]
pub enum MidiMapMessage {
    /// Right-click on a mappable control: open its MIDI menu at `x`/`y`
    /// (window coordinates).
    OpenMenu { target: MidiTarget, x: f32, y: f32 },
    /// Backdrop click, Esc, or after an entry ran.
    CloseMenu,
    /// Arm learn for `target`: the next control moved on the control
    /// surface is bound to it. Arming the target already armed cancels.
    Learn(MidiTarget),
    /// Disarm learn without binding anything (Esc, the Cancel chip).
    CancelLearn,
    /// The engine captured `source` for the armed `target` — bind it,
    /// replacing whatever that control drove before.
    Bind { target: MidiTarget, source: ControlSource },
    /// Remove one binding.
    Clear(BindingId),
    /// Remove every binding that drives `target`.
    ClearTarget(MidiTarget),
    /// Remove every binding in the project.
    ClearAll,
    /// Replace the project's bindings with a saved controller map's.
    LoadControllerMap(String),
    /// Pick (`Some`) or clear the control-surface input port. A user
    /// setting: it is the machine's hardware, not the project's.
    SetControlSurfaceInput(Option<String>),
    /// The Settings → MIDI "save as map" name field.
    SetMapName(String),
    /// Save the project's bindings as a controller map under the typed
    /// name (replacing a map of that name).
    SaveControllerMap,
    /// Delete a saved controller map.
    DeleteControllerMap(String),
}

impl MidiMapMessage {
    /// Undo classification (ARCH-06 A6-4). Binding edits change the
    /// project; arming learn, the menu, the port and the preset files do
    /// not.
    pub(crate) fn undo_action(&self) -> UndoAction {
        match self {
            Self::Bind { .. }
            | Self::Clear(_)
            | Self::ClearTarget(_)
            | Self::ClearAll
            | Self::LoadControllerMap(_) => UndoAction::Record,
            Self::OpenMenu { .. }
            | Self::CloseMenu
            | Self::Learn(_)
            | Self::CancelLearn
            | Self::SetControlSurfaceInput(_)
            | Self::SetMapName(_)
            | Self::SaveControllerMap
            | Self::DeleteControllerMap(_) => UndoAction::Skip,
        }
    }

    /// The undo-history label (`edit.status`).
    pub(crate) fn describe(&self) -> &'static str {
        match self {
            Self::Bind { .. } => "learn MIDI binding",
            Self::Clear(_) | Self::ClearTarget(_) => "clear MIDI binding",
            Self::ClearAll => "clear all MIDI bindings",
            Self::LoadControllerMap(_) => "load controller map",
            Self::OpenMenu { .. }
            | Self::CloseMenu
            | Self::Learn(_)
            | Self::CancelLearn
            | Self::SetControlSurfaceInput(_)
            | Self::SetMapName(_)
            | Self::SaveControllerMap
            | Self::DeleteControllerMap(_) => "MIDI mapping",
        }
    }
}

pub(crate) fn handle(r: &mut Resonance, msg: MidiMapMessage) -> Task<Message> {
    match msg {
        MidiMapMessage::OpenMenu { target, x, y } => {
            r.devices.midi_map.menu = Some(MidiMenuState { target, x, y });
        }
        MidiMapMessage::CloseMenu => r.devices.midi_map.menu = None,
        MidiMapMessage::Learn(target) => {
            r.devices.midi_map.menu = None;
            if r.devices.midi_map.learn_target == Some(target) {
                cancel_learn(r);
            } else {
                r.devices.midi_map.learn_target = Some(target);
                let _ = r.engine.send(AudioCommand::EnterMidiLearn { target });
            }
        }
        MidiMapMessage::CancelLearn => cancel_learn(r),
        MidiMapMessage::Bind { target, source } => bind(r, target, source),
        MidiMapMessage::Clear(id) => clear(r, &[id]),
        MidiMapMessage::ClearTarget(target) => {
            r.devices.midi_map.menu = None;
            let ids: Vec<BindingId> =
                r.devices.midi_map.for_target(target).iter().map(|b| b.id).collect();
            clear(r, &ids);
        }
        MidiMapMessage::ClearAll => {
            r.devices.midi_map.replace_all([]);
            let _ = r.engine.send(AudioCommand::ClearAllMidiBindings);
        }
        MidiMapMessage::LoadControllerMap(name) => {
            if let Some(map) = r.devices.midi_map.saved_maps.iter().find(|m| m.name == name) {
                let map = map.clone();
                set_all(r, map.bindings);
            }
        }
        MidiMapMessage::SetControlSurfaceInput(device) => {
            r.settings.midi.control_surface_input = device.clone();
            crate::settings::persist(&r.settings);
            let _ = r.engine.send(AudioCommand::SetControlSurfaceInput { device });
        }
        MidiMapMessage::SetMapName(name) => r.devices.midi_map.map_name = name,
        MidiMapMessage::SaveControllerMap => {
            let name = r.devices.midi_map.map_name.trim().to_string();
            if !name.is_empty() {
                let map = ControllerMap {
                    name,
                    bindings: r.devices.midi_map.sorted(),
                };
                if let Err(e) = resonance_common::midi_map::save_controller_map_to(&map, &maps_path()) {
                    r.banners.error_message = Some(format!("Saving the controller map failed: {e}"));
                }
                r.devices.midi_map.map_name.clear();
                reload_saved_maps(r);
            }
        }
        MidiMapMessage::DeleteControllerMap(name) => {
            if let Err(e) = resonance_common::midi_map::delete_controller_map_from(&name, &maps_path()) {
                r.banners.error_message = Some(format!("Deleting the controller map failed: {e}"));
            }
            reload_saved_maps(r);
        }
    }
    Task::none()
}

/// Where the controller-map presets live: `<data dir>/resonance/
/// controller_maps.json` (the hermetic data dir in a test process).
pub(crate) fn maps_path() -> std::path::PathBuf {
    crate::user_dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("resonance/controller_maps.json")
}

/// Re-read the controller-map presets (Settings → MIDI shows them).
pub(crate) fn reload_saved_maps(r: &mut Resonance) {
    r.devices.midi_map.saved_maps = resonance_common::midi_map::load_controller_maps_from(&maps_path());
}

fn cancel_learn(r: &mut Resonance) {
    if r.devices.midi_map.learn_target.take().is_some() {
        let _ = r.engine.send(AudioCommand::CancelMidiLearn);
    }
}

/// Bind `source` to `target` with the default range / mode / takeover.
/// A control drives one target, so whatever binding it had goes — the
/// engine displaces it the same way when `SetMidiBinding` arrives.
fn bind(r: &mut Resonance, target: MidiTarget, source: ControlSource) {
    let map = &mut r.devices.midi_map;
    for id in map.on_control(source) {
        map.clear(id);
    }
    let binding = MidiBinding::new(map.alloc_id(), source, target);
    map.upsert(binding);
    if map.learn_target == Some(target) {
        map.learn_target = None;
    }
    let _ = r.engine.send(AudioCommand::SetMidiBinding { binding });
}

fn clear(r: &mut Resonance, ids: &[BindingId]) {
    for &id in ids {
        if r.devices.midi_map.bindings.contains_key(&id) {
            r.devices.midi_map.clear(id);
            let _ = r.engine.send(AudioCommand::ClearMidiBinding { id });
        }
    }
}

/// Replace the project's bindings wholesale, here and in the engine (a
/// controller map, a project load, an undo).
pub(crate) fn set_all(r: &mut Resonance, bindings: Vec<MidiBinding>) {
    r.devices.midi_map.replace_all(bindings.iter().copied());
    let _ = r.engine.send(AudioCommand::SetControllerMap {
        map: ControllerMap {
            name: "project".to_string(),
            bindings,
        },
    });
}

// ---------------------------------------------------------------------------
// Applying a hardware move
// ---------------------------------------------------------------------------

/// The message a control-surface move on `binding` sends, `value` being
/// the raw CC value / note-on velocity — or `None` when it changes
/// nothing: its target is gone, soft takeover is still waiting for the
/// control to reach the current value, or a toggle's CC sent a release.
pub(crate) fn move_message(r: &Resonance, binding: &MidiBinding, value: u8) -> Option<Message> {
    let is_note = matches!(binding.source, ControlSource::Note { .. });
    // A toggle or trigger fires on a press: a note-on (the engine never
    // reports a release), or a CC button's upper half.
    let pressed = is_note || value >= 64;
    match binding.target {
        MidiTarget::TrackMute(id) => {
            let track = r.registry.tracks.iter().find(|t| t.id == id)?;
            toggle_to(is_note, pressed, track.muted).then_some(Message::Track(TrackMessage::ToggleMute(id)))
        }
        MidiTarget::TrackSolo(id) => {
            let track = r.registry.tracks.iter().find(|t| t.id == id)?;
            toggle_to(is_note, pressed, track.soloed).then_some(Message::Track(TrackMessage::ToggleSolo(id)))
        }
        MidiTarget::Transport(action) => pressed.then_some(Message::Transport(match action {
            TransportAction::Play => TransportMessage::Play,
            TransportAction::Stop => TransportMessage::Stop,
            TransportAction::Record => TransportMessage::Record,
            TransportAction::LoopToggle => TransportMessage::ToggleLoop,
        })),
        MidiTarget::TrackVolume(id) => {
            let track = r.registry.tracks.iter().find(|t| t.id == id)?;
            let lane = AutomationTarget::TrackGain(id);
            let norm = continuous(binding, value, real_to_lane_value(&lane, track.volume))?;
            Some(Message::Track(TrackMessage::SetTrackVolume(id, lane_value_to_real(&lane, norm))))
        }
        MidiTarget::TrackPan(id) => {
            let track = r.registry.tracks.iter().find(|t| t.id == id)?;
            let lane = AutomationTarget::TrackPan(id);
            let norm = continuous(binding, value, real_to_lane_value(&lane, track.pan))?;
            Some(Message::Track(TrackMessage::SetTrackPan(id, lane_value_to_real(&lane, norm))))
        }
        MidiTarget::SendLevel { send, .. } => {
            let s = r.aux.sends.iter().find(|s| s.id == send.0)?;
            // Sends share the fader's dB scale.
            let lane = AutomationTarget::MasterGain;
            let norm = continuous(binding, value, real_to_lane_value(&lane, s.level_db))?;
            Some(Message::Mixer(MixerMessage::SetSendLevel(s.id, lane_value_to_real(&lane, norm))))
        }
        MidiTarget::PluginParam { instance, param_id } => {
            let plugin = r.plugin_slot(instance)?;
            let p = plugin.params.iter().find(|p| p.id == param_id && !p.read_only)?;
            let span = p.max_value - p.min_value;
            let current = if span > 0.0 {
                ((p.current_value - p.min_value) / span) as f32
            } else {
                0.0
            };
            let norm = continuous(binding, value, current)?;
            let mut real = lane_value_to_plugin_param(norm, p.min_value, p.max_value);
            if p.stepped {
                real = real.round();
            }
            Some(Message::Plugin(PluginMessage::SetPluginParam(instance, param_id, real)))
        }
    }
}

/// Whether a press flips a toggle that is `on` now. A note (or a CC
/// button read as a note) flips on every press; a CC switch sends its
/// state — on in the upper half, off in the lower — so it only flips when
/// that differs.
fn toggle_to(is_note: bool, pressed: bool, on: bool) -> bool {
    if is_note {
        pressed
    } else {
        pressed != on
    }
}

/// The normalized value a continuous target moves to, through the
/// binding's mode, range and soft takeover; `None` while takeover holds.
fn continuous(binding: &MidiBinding, value: u8, current: f32) -> Option<f32> {
    let incoming = match binding.source {
        ControlSource::Cc {
            mode: CcMode::Relative(enc),
            ..
        } => {
            // An encoder moves from where the target is: nothing to take
            // over.
            return Some(apply_delta(
                current,
                decode_relative(enc, value),
                binding.min,
                binding.max,
                binding.invert,
            ));
        }
        _ => cc_to_norm(value, binding.min, binding.max, binding.invert),
    };
    takeover_value(binding.takeover, incoming, current)
}

/// Handle `ControlSurfaceMoved`: run the move's message through the full
/// update path (undo coalescing, dirty flag, the engine command).
pub(crate) fn hardware_moved(r: &mut Resonance, binding: MidiBinding, value: u8) -> Task<Message> {
    match move_message(r, &binding, value) {
        Some(message) => r.update(message),
        None => Task::none(),
    }
}

/// Esc while learning disarms (not under a modal overlay, which Esc
/// closes first). Returns whether it did.
pub(crate) fn escape(r: &mut Resonance) -> bool {
    if r.devices.midi_map.learn_target.is_none() {
        return false;
    }
    cancel_learn(r);
    true
}

