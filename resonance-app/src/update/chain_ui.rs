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
            let switching = r.ui.mixer.popover_open();
            r.ui.mixer.dismiss_inspector_popovers();
            if !open && r.plugin_slot(instance_id).is_some() {
                r.ui.mixer.slot_menu = Some(instance_id);
                r.ui.mixer.popover_switched = switching;
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
        ChainUiMessage::Dismiss => {
            // The press that switched one popover for another reaches
            // the click-away listener too, after the switch itself.
            if std::mem::take(&mut r.ui.mixer.popover_switched) {
                return Task::none();
            }
            r.ui.mixer.dismiss_inspector_popovers();
        }
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
        ChainUiMessage::BeginPresetSave(instance_id) => return begin_preset_save(r, instance_id),
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
                // A drag still armed here lost its release; the press
                // listener sees this same press next and must not disarm
                // the new one.
                let rearmed = r.ui.mixer.chain_drag.is_some();
                r.ui.mixer.dismiss_inspector_popovers();
                r.ui.mixer.chain_drag = Some(ChainDragState {
                    instance_id,
                    over: None,
                    rearmed,
                });
            }
        }
        ChainUiMessage::DragOver(instance_id) => {
            if let Some(drag) = r.ui.mixer.chain_drag.as_mut() {
                drag.over = Some(instance_id);
            }
        }
        ChainUiMessage::DragLeave(instance_id) => {
            // Keyed by row: moving straight from one row to the next
            // delivers the next row's enter and this row's exit in tree
            // order, and the exit must not clear the row just entered.
            if let Some(drag) = r.ui.mixer.chain_drag.as_mut() {
                if drag.over == Some(instance_id) {
                    drag.over = None;
                }
            }
        }
        ChainUiMessage::DragPointerPressed => {
            match r.ui.mixer.chain_drag.as_mut() {
                Some(drag) if drag.rearmed => drag.rearmed = false,
                Some(_) => r.ui.mixer.chain_drag = None,
                None => {}
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
            let switching = r.ui.mixer.popover_open();
            r.ui.mixer.dismiss_inspector_popovers();
            if !open {
                r.ui.mixer.color_palette = Some(track_id);
                r.ui.mixer.popover_switched = switching;
            }
        }
        ChainUiMessage::CueInstrumentPicker(track_id) => {
            // Select the track the way a click on its strip does, then
            // make sure its CHAIN group is open and cue the picker.
            let task = r.update(Message::Ui(crate::message::UiMessage::SelectTrack(Some(
                track_id,
            ))));
            let lacks = r
                .registry
                .tracks
                .iter()
                .find(|t| t.id == track_id)
                .is_some_and(|t| crate::plugin_chain::lacks_instrument(r, t));
            if lacks {
                r.ui
                    .mixer
                    .collapsed_inspector_groups
                    .remove(&crate::state::MixerInspectorGroup::Chain);
                r.ui.mixer.instrument_picker_cue = Some(track_id);
            }
            return task;
        }
    }
    Task::none()
}

/// While a CHAIN-row drag is armed: the window-level events that end it.
/// The release drops onto the row under the pointer, if any (rows
/// record it on enter and clear it on exit, so a release off every row
/// moves nothing); the window losing focus means the release went
/// elsewhere; a press means the release was lost (`DragPointerPressed`).
pub fn drag_end_event(event: &iced::Event) -> Option<Message> {
    let ui = |m| Some(Message::Plugin(PluginMessage::ChainUi(m)));
    match event {
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
            ui(ChainUiMessage::DragDrop)
        }
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_)) => {
            ui(ChainUiMessage::DragPointerPressed)
        }
        iced::Event::Window(iced::window::Event::Unfocused) => ui(ChainUiMessage::DragCancel),
        _ => None,
    }
}

/// While a slot menu or the colour palette is open: any press is a
/// click-away (`ChainUiMessage::Dismiss`). A press on one of the
/// popover's own entries has already closed it through `Pick` by the
/// time this arrives, and one that opened another popover is spent by
/// `popover_switched`.
pub fn popover_press_event(event: &iced::Event) -> Option<Message> {
    match event {
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_))
        | iced::Event::Touch(iced::touch::Event::FingerPressed { .. }) => {
            Some(Message::Plugin(PluginMessage::ChainUi(ChainUiMessage::Dismiss)))
        }
        _ => None,
    }
}

/// Esc closes the CHAIN rows' transient state, one layer per press and
/// the most recent kind first: a drag, then the preset-name prompt, then
/// replace mode, then an open slot menu / colour palette / picker cue.
/// Returns whether Esc closed something.
pub(crate) fn escape(r: &mut Resonance) -> bool {
    let mixer = &mut r.ui.mixer;
    if mixer.chain_drag.take().is_some() {
        return true;
    }
    if mixer.slot_preset_save.take().is_some() {
        return true;
    }
    if mixer.replacing_slot.take().is_some() {
        return true;
    }
    if mixer.popover_open() || mixer.instrument_picker_cue.is_some() {
        mixer.dismiss_inspector_popovers();
        mixer.instrument_picker_cue = None;
        return true;
    }
    false
}

/// Open the "Save preset…" prompt for a loaded slot, seeded with the
/// loaded preset's name (or the plugin's, when none is loaded), and put
/// the caret in it with the name selected, so typing replaces it and
/// Enter saves.
fn begin_preset_save(r: &mut Resonance, instance_id: PluginInstanceId) -> Task<Message> {
    let Some(slot) = r.plugin_slot(instance_id) else {
        return Task::none();
    };
    if slot.availability.reason().is_some() {
        return Task::none();
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
    let id = crate::view::mixer::inspector::chain::preset_name_input_id();
    Task::batch([
        iced::widget::operation::focus(id.clone()),
        iced::widget::operation::select_all(id),
    ])
}

/// Arm the capture through the same path `*.save_plugin_preset` takes,
/// so the GUI and the control API write the same file.
///
/// Refused, with the prompt left open, while a save of the same slot is
/// still waiting on the plugin's state: arming again would replace that
/// pending save — possibly a control-API one — before it is written. And
/// "does this name exist" is asked again here, not trusted from the last
/// keystroke: a preset of that name saved since then (by the control API,
/// or a save that just landed) turns the button into "Overwrite" and
/// needs a second press, rather than being overwritten unannounced.
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
    if r
        .presets
        .pending_plugin_preset_saves
        .contains_key(&prompt.instance_id)
    {
        r.banners.error_message = Some(
            "A preset save for this plugin is still in progress \u{2014} try again in a moment"
                .to_string(),
        );
        return;
    }
    let exists = user_preset_exists(r, &clap_id, &prompt.name);
    if exists && !prompt.exists {
        if let Some(p) = r.ui.mixer.slot_preset_save.as_mut() {
            p.exists = true;
        }
        return;
    }
    let args = crate::update::control::plugin_presets::SaveArgs {
        name: prompt.name.clone(),
        // The button read "Overwrite" when the press was made and the
        // name is still taken — the confirmation the control API asks
        // for. A name that is free saves as new.
        overwrite: exists,
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
