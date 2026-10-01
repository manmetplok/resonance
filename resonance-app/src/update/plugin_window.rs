//! "Open always opens a window" (mixer-cleanup.md §4): the routing of
//! `PluginMessage::OpenPluginWindow`, and the generic window's drag and
//! Esc handling. The window itself is `view/plugin_window.rs`; its state is
//! [`crate::state::PluginWindowState`].

use iced::Task;
use resonance_audio::types::PluginInstanceId;

use crate::message::{Message, PluginMessage, PluginWindowDrag};
use crate::state::{PluginWindowState, ViewMode, PLUGIN_WINDOW_DEFAULT_POSITION};
use crate::Resonance;

/// Open `instance_id`'s window: its own editor when it has a GUI and is
/// there to show one, the host-drawn generic window otherwise. A missing
/// or unavailable plugin always gets the generic window — it is where the
/// recovery (what is missing, the replace picker) lives.
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

/// Show the generic window for `instance_id`. An open window keeps its
/// place and takes the new plugin; a fresh one lands at the default spot.
pub(crate) fn open_generic(r: &mut Resonance, instance_id: PluginInstanceId) {
    let position = r
        .ui
        .mixer
        .plugin_window
        .map_or(PLUGIN_WINDOW_DEFAULT_POSITION, |w| w.position);
    r.ui.mixer.plugin_window = Some(PluginWindowState::new(instance_id, position));
}

/// One step of the title-bar drag.
pub(crate) fn drag(r: &mut Resonance, step: PluginWindowDrag) {
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
                Some(grab) => {
                    let p = at - grab;
                    // Keep the title bar reachable: never above or left of
                    // the window's own edge.
                    window.position = iced::Point::new(p.x.max(0.0), p.y.max(0.0));
                }
            }
        }
        PluginWindowDrag::End => window.drag = None,
    }
}

/// Whether the generic window is on screen. Performance mode is a
/// full-bleed surface of its own and does not draw it.
pub(crate) fn visible(r: &Resonance) -> bool {
    r.ui.mixer.plugin_window.is_some() && !matches!(r.ui.view_mode, ViewMode::Performance)
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
