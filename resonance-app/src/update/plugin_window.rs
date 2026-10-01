//! "Open always opens a window" (mixer-cleanup.md §4): the routing of
//! `PluginMessage::OpenPluginWindow` / `OpenGenericParams`, the focused
//! slot (`MixerUiState::focused_slot`, §2.1) they set, and the generic
//! window's drag and Esc handling. The window itself is
//! `view/plugin_window.rs`; its state is
//! [`crate::state::PluginWindowState`].

use iced::Task;
use resonance_audio::types::PluginInstanceId;

use crate::message::{Message, PluginMessage, PluginWindowDrag};
use crate::state::{PluginLocator, PluginWindowState, ViewMode, PLUGIN_WINDOW_DEFAULT_POSITION};
use crate::Resonance;

/// Open `instance_id`'s window: its own editor when it has a GUI and is
/// there to show one, the host-drawn generic window otherwise. A missing
/// or unavailable plugin always gets the generic window — it is where the
/// recovery (what is missing, the replace picker) lives. Either way the
/// slot becomes the focused one, so the preset commands act on it.
pub(crate) fn open(r: &mut Resonance, instance_id: PluginInstanceId) -> Task<Message> {
    let Some(slot) = r.plugin_slot(instance_id) else {
        return Task::none();
    };
    if slot.has_gui && slot.availability.reason().is_none() {
        return r.update(Message::Plugin(PluginMessage::OpenPluginEditor(
            instance_id,
        )));
    }
    open_generic(r, instance_id);
    Task::none()
}

/// Show the generic window for `instance_id` and focus the slot. An open
/// window keeps its place and takes the new plugin; a fresh one lands at
/// the default spot, as does one whose stored place is no longer on
/// screen. A slot that does not exist (an editor-failure echo that
/// raced the slot's removal) opens nothing.
pub(crate) fn open_generic(r: &mut Resonance, instance_id: PluginInstanceId) {
    if r.plugin_slot(instance_id).is_none() {
        return;
    }
    r.ui.mixer.focused_slot = Some(instance_id);
    let viewport = r.ui.window_size;
    let position = r
        .ui
        .mixer
        .plugin_window
        .map(|w| w.position)
        .filter(|p| PluginWindowState::title_on_screen(*p, viewport))
        .unwrap_or(PLUGIN_WINDOW_DEFAULT_POSITION);
    r.ui.mixer.plugin_window = Some(PluginWindowState::new(instance_id, position));
}

/// Make `instance_id` the focused slot and select the channel it sits
/// on (`PluginMessage::FocusSlot`). A slot that does not exist changes
/// nothing.
///
/// Focusing a slot is not a selection gesture, so it never thins out a
/// multi-track selection: a track already in the selection just becomes
/// the primary one (the inspector's), and an additive (Cmd/Shift) click
/// adds its track to the selection. A plain click on a slot of a track
/// outside the selection selects that track alone, as a click on its
/// strip does.
pub(crate) fn focus(r: &mut Resonance, instance_id: PluginInstanceId) {
    let Some((owner, _)) = crate::update::plugin_replace::locate_slot(r, instance_id) else {
        return;
    };
    match owner {
        PluginLocator::Track(track_id) => {
            let selected = r.ui.interaction.selected_tracks.contains(&track_id);
            if selected || r.ui.interaction.select_additive {
                if r.ui.interaction.selected_track != Some(track_id) {
                    // The inspector changes owner: drop the old owner's
                    // transient CHAIN state, as `UiMessage::SelectTrack`
                    // does.
                    r.ui.mixer.reset_chain_ui();
                }
                r.ui.clear_channel_selection();
                r.ui.interaction.make_primary_track(track_id);
            } else {
                let _ = r.update(Message::Ui(crate::message::UiMessage::SelectTrack(Some(
                    track_id,
                ))));
            }
        }
        PluginLocator::Bus(bus_id) => {
            let _ = r.update(Message::Ui(crate::message::UiMessage::SelectBus(Some(bus_id))));
        }
        PluginLocator::Master => {
            let _ = r.update(Message::Ui(crate::message::UiMessage::SelectMaster));
        }
    }
    r.ui.mixer.focused_slot = Some(instance_id);
}

/// One step of the title-bar drag.
pub(crate) fn drag(r: &mut Resonance, step: PluginWindowDrag) {
    let viewport = r.ui.window_size;
    let Some(window) = r.ui.mixer.plugin_window.as_mut() else {
        return;
    };
    match step {
        PluginWindowDrag::Begin => {
            window.drag = Some(crate::state::PluginWindowDragState { grab: None });
        }
        PluginWindowDrag::Moved(at) => {
            let Some(drag) = window.drag.as_mut() else {
                return;
            };
            match drag.grab {
                // The first move after the press fixes where the window
                // was grabbed.
                None => drag.grab = Some(at - window.position),
                // Keep the title bar reachable: never above or left of
                // the app window's edge, and never so far right or down
                // that less than a grab strip of it is left on screen.
                Some(grab) => window.position = PluginWindowState::clamp(at - grab, viewport),
            }
        }
        PluginWindowDrag::End => window.drag = None,
    }
}

/// While a title-bar drag is on: the window-level event that ends it —
/// the app window losing focus, after which its button release goes
/// elsewhere and the panel would stay stuck to the pointer.
pub fn drag_end_event(event: &iced::Event) -> Option<Message> {
    matches!(event, iced::Event::Window(iced::window::Event::Unfocused))
        .then_some(Message::Plugin(PluginMessage::PluginWindowDrag(PluginWindowDrag::End)))
}

/// The app window was opened or resized to `size`: remember it (the
/// drag clamps to it) and pull an open generic window back on screen if
/// the new size left its title bar outside.
pub(crate) fn viewport_resized(r: &mut Resonance, size: iced::Size) {
    r.ui.window_size = size;
    if let Some(window) = r.ui.mixer.plugin_window.as_mut() {
        window.position = PluginWindowState::clamp(window.position, size);
    }
}

/// Whether the generic window is on screen: one is open, its slot still
/// resolves, and the view draws it — Performance mode is a full-bleed
/// surface of its own and does not.
pub(crate) fn visible(r: &Resonance) -> bool {
    !matches!(r.ui.view_mode, ViewMode::Performance)
        && r
            .ui
            .mixer
            .plugin_window_id()
            .is_some_and(|id| r.plugin_slot(id).is_some())
}

/// Esc with no modal overlay up closes the generic window, before the key
/// means anything else. Returns whether it did.
pub(crate) fn escape(r: &mut Resonance) -> bool {
    if !visible(r) {
        return false;
    }
    r.ui.mixer.plugin_window = None;
    true
}

/// The slot the preset commands (◀ / ▶ / browse) and the media tab's
/// double-click load act on: the focused slot, while it still resolves
/// to a plugin. `None` in Performance mode, where neither the window nor
/// the inspector is on screen — a step there would change a sound the
/// user cannot see is targeted.
pub(crate) fn preset_target(r: &Resonance) -> Option<PluginInstanceId> {
    if matches!(r.ui.view_mode, ViewMode::Performance) {
        return None;
    }
    r.ui
        .mixer
        .focused_slot
        .filter(|id| r.plugin_slot(*id).is_some())
}
