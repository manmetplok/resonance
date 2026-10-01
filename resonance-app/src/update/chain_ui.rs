//! The inspector CHAIN rows' and header's own gestures
//! (`PluginMessage::ChainUi`, mixer-cleanup.md §3.2 / §3.1 and slice
//! S7): the ☰ slot menu, replace mode, the "Save preset…" prompt, drag
//! reorder and the track-colour palette.
//!
//! Everything here is view state. The edits these gestures lead to — a
//! move, a remove, a replace, a colour — are dispatched as their own
//! ordinary messages through `Resonance::update`, so each takes the undo
//! entry and runs the gates it always has; nothing is re-implemented.

use iced::Task;
use resonance_audio::types::PluginInstanceId;

use crate::message::{ChainUiMessage, Message, PluginMessage, PresetUiMessage};
use crate::state::{ChainDragState, SlotPresetSaveState};
use crate::Resonance;

pub(crate) fn update(r: &mut Resonance, msg: ChainUiMessage) -> Task<Message> {
    match msg {
        ChainUiMessage::ToggleSlotMenu(instance_id) => {
            let open = r.ui.mixer.slot_menu == Some(instance_id);
            r.ui.mixer.dismiss_inspector_popovers();
            if !open && r.plugin_slot(instance_id).is_some() {
                r.ui.mixer.slot_menu = Some(instance_id);
            }
        }
        ChainUiMessage::Pick(message) => {
            r.ui.mixer.dismiss_inspector_popovers();
            // A pick ends replace mode too: the replace picker's own
            // choice comes through here, and so does any other menu
            // entry, which means the user moved on.
            r.ui.mixer.replacing_slot = None;
            return r.update(*message);
        }
        ChainUiMessage::Dismiss => r.ui.mixer.dismiss_inspector_popovers(),
        ChainUiMessage::BrowsePresets(instance_id) => {
            // Focus first, so the browser's ◀ / ▶ and the media tab's
            // load land on the slot the browser was opened for.
            crate::update::plugin_window::focus(r, instance_id);
            return r.update(Message::Plugin(PluginMessage::PresetUi(
                PresetUiMessage::OpenBrowser(instance_id),
            )));
        }
        ChainUiMessage::BeginReplace(instance_id) => {
            if r.plugin_slot(instance_id).is_some() {
                r.ui.mixer.replacing_slot = Some(instance_id);
            }
        }
        ChainUiMessage::CancelReplace => r.ui.mixer.replacing_slot = None,
        ChainUiMessage::BeginPresetSave(instance_id) => begin_preset_save(r, instance_id),
        ChainUiMessage::PresetSaveName(name) => {
            let clap_id = r
                .ui
                .mixer
                .slot_preset_save
                .as_ref()
                .and_then(|p| r.plugin_slot(p.instance_id))
                .map(|slot| slot.clap_plugin_id.clone());
            if let (Some(prompt), Some(clap_id)) = (r.ui.mixer.slot_preset_save.as_ref(), clap_id) {
                let exists = user_preset_exists(r, &clap_id, &name);
                let instance_id = prompt.instance_id;
                r.ui.mixer.slot_preset_save = Some(SlotPresetSaveState {
                    instance_id,
                    name,
                    exists,
                });
            }
        }
        ChainUiMessage::CommitPresetSave => commit_preset_save(r),
        ChainUiMessage::CancelPresetSave => r.ui.mixer.slot_preset_save = None,
        ChainUiMessage::DragStart(instance_id) => {
            if r.plugin_slot(instance_id).is_some() {
                r.ui.mixer.dismiss_inspector_popovers();
                r.ui.mixer.chain_drag = Some(ChainDragState {
                    instance_id,
                    over: None,
                });
            }
        }
        ChainUiMessage::DragOver(instance_id) => {
            if let Some(drag) = r.ui.mixer.chain_drag.as_mut() {
                drag.over = Some(instance_id);
            }
        }
        ChainUiMessage::DragDrop => {
            let Some(drag) = r.ui.mixer.chain_drag.take() else {
                return Task::none();
            };
            let Some(onto) = drag.over else {
                return Task::none();
            };
            if let Some(m) = crate::view::mixer::reorder::drop_move(r, drag.instance_id, onto) {
                return r.update(m);
            }
        }
        ChainUiMessage::DragCancel => r.ui.mixer.chain_drag = None,
        ChainUiMessage::ToggleColorPalette(track_id) => {
            let open = r.ui.mixer.color_palette == Some(track_id);
            r.ui.mixer.dismiss_inspector_popovers();
            if !open {
                r.ui.mixer.color_palette = Some(track_id);
            }
        }
    }
    Task::none()
}

/// While a CHAIN-row drag is armed: the window-level events that end it.
/// The release anywhere drops (the hovered row was recorded on the way
/// in); the window losing focus means the release went elsewhere.
pub fn drag_end_event(event: &iced::Event) -> Option<Message> {
    let ui = |m| Some(Message::Plugin(PluginMessage::ChainUi(m)));
    match event {
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
            ui(ChainUiMessage::DragDrop)
        }
        iced::Event::Window(iced::window::Event::Unfocused) => ui(ChainUiMessage::DragCancel),
        _ => None,
    }
}

/// Open the "Save preset…" prompt for a loaded slot, seeded with the
/// loaded preset's name (or the plugin's, when none is loaded).
fn begin_preset_save(r: &mut Resonance, instance_id: PluginInstanceId) {
    let Some(slot) = r.plugin_slot(instance_id) else {
        return;
    };
    if slot.availability.reason().is_some() {
        return;
    }
    let clap_id = slot.clap_plugin_id.clone();
    let name = r
        .presets
        .plugin_preset_identity
        .get(&instance_id)
        .map(|i| i.name.clone())
        .unwrap_or_else(|| slot.plugin_name.clone());
    let exists = user_preset_exists(r, &clap_id, &name);
    r.ui.mixer.dismiss_inspector_popovers();
    r.ui.mixer.slot_preset_save = Some(SlotPresetSaveState {
        instance_id,
        name,
        exists,
    });
}

/// Arm the capture through the same path `*.save_plugin_preset` takes,
/// so the GUI and the control API write the same file.
fn commit_preset_save(r: &mut Resonance) {
    let Some(prompt) = r.ui.mixer.slot_preset_save.clone() else {
        return;
    };
    let Some(clap_id) = r
        .plugin_slot(prompt.instance_id)
        .map(|s| s.clap_plugin_id.clone())
    else {
        r.ui.mixer.slot_preset_save = None;
        return;
    };
    if prompt.name.trim().is_empty() {
        return;
    }
    let args = crate::update::control::plugin_presets::SaveArgs {
        name: prompt.name.clone(),
        // The button said "Overwrite" before the click when the name is
        // taken, which is the confirmation the control API asks for.
        overwrite: true,
        meta: None,
        favorite: None,
        overwrite_id: None,
    };
    match crate::update::control::plugin_presets::arm_save(r, clap_id, prompt.instance_id, args) {
        Ok(_) => r.ui.mixer.slot_preset_save = None,
        Err(e) => {
            r.banners.error_message = Some(format!("Could not save preset: {}", e.message));
        }
    }
}

fn user_preset_exists(r: &Resonance, clap_id: &str, name: &str) -> bool {
    let wanted = name.trim();
    crate::update::control::plugin_presets::bank_for(r, clap_id)
        .list_user()
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case(wanted))
}
