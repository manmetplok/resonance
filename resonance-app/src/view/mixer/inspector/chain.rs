//! CHAIN group — the plugin rows and the functional "+ Add to chain" /
//! "+ Add instrument" picker for the mixer inspector.
//!
//! One row anatomy serves the track, bus and master chains
//! (mixer-cleanup.md §3.2):
//!
//! ```text
//! ⠿ ● Resonance Wave            ↗  ☰  ×
//! ```
//!
//! `⠿` drags the row (slice S7, rules in [`reorder::drop_move`]); `●`
//! toggles bypass (`○` when bypassed, BAD-pink when the plugin is
//! missing); the name focuses the slot on a click and opens it on a
//! double-click; `↗` opens it; `☰` opens the slot menu (parameters,
//! presets, replace, move, remove) under the row; `×` removes it. A
//! missing plugin carries its recovery actions inline under its row.
//!
//! Everything here is drawn inside the owner's lazy body, so every
//! input it reads is hashed by [`hash_chain_ui`].

use iced::widget::{
    button, column, container, mouse_area, pick_list, row, text, text_input, Space,
};
use iced::{alignment, Element, Length};
use resonance_audio::types::{PluginInstanceId, ScannedPlugin};

use crate::message::{
    ChainUiMessage, Message, PluginMessage, PresetAddOwner, PresetUiMessage,
};
use crate::state::{PluginSlotState, TrackState};
use crate::theme;
use crate::view::mixer::picks::PluginOwner;
use crate::view::mixer::reorder;

/// Font Awesome Solid `grip-vertical` — the row's drag handle.
const GLYPH_GRIP: char = '\u{f58e}';
/// Font Awesome Solid `up-right-from-square` — "open window".
const GLYPH_OPEN: char = '\u{f35d}';
/// Font Awesome Solid `bars` — the slot menu.
const GLYPH_MENU: char = theme::fa::BARS;

/// Longest plugin name a CHAIN row prints before the ellipsis. The row
/// also clips, so this only decides where the "…" goes.
const ROW_NAME_CHARS: usize = 22;

pub(super) fn chain_group(
    r: &crate::Resonance,
    track: &TrackState,
    collapsed: bool,
) -> Element<'static, Message> {
    if collapsed {
        return super::widgets::group_header(
            "CHAIN",
            crate::state::MixerInspectorGroup::Chain,
            true,
        );
    }

    // 10px column spacing doubles as the title → first-row gap, so no
    // explicit spacer is needed after the group title.
    let mut col = column![super::widgets::group_header(
        "CHAIN",
        crate::state::MixerInspectorGroup::Chain,
        false,
    )]
    .spacing(10);

    // The instrument slot is drawn where the chain actually holds the
    // instrument (`plugin_chain::displayed_instrument_slot`), never
    // "row 0 because this is an instrument track": a sub-track's chain
    // is effects only, an external instrument's synth is outboard
    // hardware (every plugin there is an insert over the audio return),
    // and an instrument track can have an effect ahead of its instrument
    // or no instrument at all.
    let instrument_index = crate::plugin_chain::displayed_instrument_slot(r, track);
    col = col.push(chain_rows(
        r,
        PluginOwner::Track(track.id),
        &track.plugins,
        instrument_index,
    ));

    if let Some(replace) = replace_picker(r, &track.plugins) {
        return col.push(replace).into();
    }

    // Functional add pickers. An instrument track without an instrument
    // gets the instrument picker first — and, once it has effects, the
    // FX picker under it, so neither kind is out of reach. Everyone else
    // gets the FX picker. A picker is skipped when nothing of its kind
    // has been scanned. Options come from
    // `view_caches.{fx,instrument}_plugins` — Rc clones, not a per-frame
    // filter pass.
    let lacks_instrument = crate::plugin_chain::lacks_instrument(r, track);
    if lacks_instrument {
        // The strip's "No instrument" line cues this picker (§2.1):
        // iced cannot focus a pick_list, so the cue is an accent border
        // and a line saying what to do.
        let cued = r.ui.mixer.instrument_picker_cue == Some(track.id);
        if cued && !r.ui.view_caches.instrument_plugins.is_empty() {
            col = col.push(
                text("Pick an instrument to play this track")
                    .size(11)
                    .color(theme::ACCENT_SOFT),
            );
        }
        col = push_add_pickers(r, col, track.id, true, cued);
    }
    if !lacks_instrument || !track.plugins.is_empty() {
        col = push_add_pickers(r, col, track.id, false, false);
    }

    col.into()
}

/// The `+ Add instrument` / `+ Add to chain` picker and its "▸ with
/// preset…" companion, pushed onto `col`. `cued` draws the picker with
/// an accent border (the strip's "No instrument" line pointed here).
fn push_add_pickers(
    r: &crate::Resonance,
    mut col: iced::widget::Column<'static, Message>,
    track_id: resonance_audio::types::TrackId,
    instrument: bool,
    cued: bool,
) -> iced::widget::Column<'static, Message> {
    let candidates = if instrument {
        r.ui.view_caches.instrument_plugins.clone()
    } else {
        r.ui.view_caches.fx_plugins.clone()
    };
    if candidates.is_empty() {
        return col;
    }
    let placeholder = if instrument {
        "+ Add instrument"
    } else {
        "+ Add to chain"
    };
    let picker = pick_list(
        candidates,
        None::<ScannedPlugin>,
        move |plugin: ScannedPlugin| {
            Message::Plugin(PluginMessage::AddPluginToTrack(track_id, plugin))
        },
    )
    .placeholder(placeholder)
    .text_size(12)
    .padding([8, 10])
    .width(Length::Fill)
    .style(move |theme, status| {
        let mut style = pick_list::default(theme, status);
        if cued {
            style.border.color = theme::ACCENT;
            style.border.width = 1.0;
        }
        style
    });
    col = col.push(picker);
    // "▸ with preset…": the user's favourite presets of the same kind
    // of plugin (§6.6), precomputed on a scan or a star.
    let picks = if instrument {
        r.presets.instrument_favorite_picks.clone()
    } else {
        r.presets.fx_favorite_picks.clone()
    };
    if !picks.is_empty() {
        let with_preset = pick_list(
            picks,
            None::<crate::state::presets::PresetAddPick>,
            move |pick| {
                Message::Plugin(PluginMessage::PresetUi(PresetUiMessage::AddWithPreset {
                    owner: PresetAddOwner::Track(track_id),
                    pick,
                }))
            },
        )
        .placeholder("\u{25b8} with preset\u{2026}")
        .text_size(12)
        .padding([8, 10])
        .width(Length::Fill);
        col = col.push(with_preset);
    }
    col
}

/// Every row of `owner`'s chain (or the "Empty chain" placeholder), with
/// the drag's drop indicator, the open slot menu, the preset prompt and
/// a missing plugin's recovery folded in under their rows.
///
/// `instrument_index` is the instrument slot, if the chain has one
/// (`plugin_chain::displayed_instrument_slot`): accent tint, and a handle
/// that does not drag (the slot is fixed).
pub(super) fn chain_rows(
    r: &crate::Resonance,
    owner: PluginOwner,
    plugins: &[PluginSlotState],
    instrument_index: Option<usize>,
) -> Element<'static, Message> {
    if plugins.is_empty() {
        return empty_chain_row();
    }
    let drag = r.ui.mixer.chain_drag;
    // Where the dragged slot would land: the hovered row's index, and
    // whether it comes from above (indicator under the row) or below.
    let dragged_index = drag.and_then(|d| plugins.iter().position(|p| p.instance_id == d.instance_id));
    let target_index = drag
        .and_then(|d| d.over)
        .and_then(|over| plugins.iter().position(|p| p.instance_id == over));
    let drop_ok = drag
        .and_then(|d| d.over.map(|over| (d.instance_id, over)))
        .is_some_and(|(dragged, over)| reorder::drop_move(r, dragged, over).is_some());

    let len = plugins.len();
    let mut col = column![].spacing(6);
    for (index, plugin) in plugins.iter().enumerate() {
        let is_instrument_slot = instrument_index == Some(index);
        let indicator = match (dragged_index, target_index) {
            (Some(from), Some(to)) if to == index && from != to && drop_ok => {
                Some(from < to)
            }
            _ => None,
        };
        if indicator == Some(false) {
            col = col.push(drop_indicator());
        }
        let row_el = chain_row(r, owner, plugin, is_instrument_slot, dragged_index == Some(index));
        // The wrapper is always there so arming a drag never changes the
        // tree's shape; it only listens while a drag is armed. Entering
        // a row makes it the drop target and leaving it clears that
        // again, so a release that lands off every row drops nothing.
        let area = mouse_area(row_el);
        let area = if drag.is_some() {
            area.on_enter(ui(ChainUiMessage::DragOver(plugin.instance_id)))
                .on_exit(ui(ChainUiMessage::DragLeave(plugin.instance_id)))
        } else {
            area
        };
        col = col.push(area);
        if indicator == Some(true) {
            col = col.push(drop_indicator());
        }
        if let Some(reason) = plugin.availability.reason() {
            col = col.push(missing_recovery(r, owner, plugin, reason));
        }
        if r.ui.mixer.slot_menu == Some(plugin.instance_id) {
            col = col.push(slot_menu(r, owner, plugin, index, len));
        }
        if let Some(prompt) = r
            .ui
            .mixer
            .slot_preset_save
            .as_ref()
            .filter(|p| p.instance_id == plugin.instance_id)
        {
            col = col.push(preset_save_prompt(prompt));
        }
    }
    col.into()
}

/// The "Empty chain" placeholder row. Shared with the bus and master
/// inspectors so every empty chain reads the same.
pub(super) fn empty_chain_row() -> Element<'static, Message> {
    super::widgets::placeholder_row("Empty chain")
}

fn ui(m: ChainUiMessage) -> Message {
    Message::Plugin(PluginMessage::ChainUi(m))
}

/// A slot-menu / palette entry: close the popover, then do `m`.
fn pick(m: Message) -> Message {
    ui(ChainUiMessage::Pick(Box::new(m)))
}

/// One plugin row: `⠿ ● Name   ↗ ☰ ×`.
fn chain_row(
    r: &crate::Resonance,
    owner: PluginOwner,
    plugin: &PluginSlotState,
    is_instrument_slot: bool,
    dragging: bool,
) -> Element<'static, Message> {
    let instance_id = plugin.instance_id;
    let missing = plugin.availability.reason().is_some();
    let bypassed = plugin.bypassed;
    let focused = r.ui.mixer.focused_slot == Some(instance_id);

    // ⠿ — drags the row. The instrument slot is fixed (the instrument
    // floor), so its handle is drawn faint and grabs nothing. A release
    // over a handle drops too: a press and its release delivered in one
    // event batch reach the handle before the window-level release
    // listener exists, and would otherwise leave the drag armed.
    let handle_color = if is_instrument_slot {
        theme::TEXT_4
    } else {
        theme::TEXT_3
    };
    let handle = container(theme::icon(GLYPH_GRIP).size(11).color(handle_color))
        .padding([2, 3]);
    let handle: Element<'static, Message> = if is_instrument_slot {
        handle.into()
    } else {
        mouse_area(handle)
            .on_press(ui(ChainUiMessage::DragStart(instance_id)))
            .on_release(ui(ChainUiMessage::DragDrop))
            .interaction(iced::mouse::Interaction::Grab)
            .into()
    };

    // ● — bypass. Sends a SET rather than a toggle so the wire and the
    // dot raise the identical message (ba todo #1305).
    let (dot, dot_color) = match (missing, bypassed) {
        (true, _) => ("\u{25cf}", theme::BAD),
        (false, true) => ("\u{25cb}", theme::TEXT_3),
        (false, false) if is_instrument_slot => ("\u{25cf}", theme::ACCENT_SOFT),
        (false, false) => ("\u{25cf}", theme::ACCENT),
    };
    let bypass = button(text(dot).size(12).color(dot_color))
        .padding([0, 3])
        .on_press(Message::Plugin(PluginMessage::SetPluginBypass {
            instance_id,
            bypassed: !bypassed,
        }))
        .style(|_theme, status| theme::ghost_button_style(status));

    // Name — click focuses, double-click opens. Single line, clipped.
    let name_color = match (missing, bypassed, is_instrument_slot) {
        (true, _, _) => theme::BAD,
        (false, true, _) => theme::TEXT_3,
        (false, false, true) => theme::ACCENT_SOFT,
        (false, false, false) => theme::TEXT_1,
    };
    let name = container(
        text(crate::util::short(&plugin.plugin_name, ROW_NAME_CHARS))
            .size(12)
            .color(name_color)
            .wrapping(iced::widget::text::Wrapping::None),
    )
    .width(Length::Fill)
    .clip(true);
    let name = mouse_area(name)
        .on_press(Message::Plugin(PluginMessage::FocusSlot(instance_id)))
        .on_double_click(Message::Plugin(PluginMessage::OpenPluginWindow(instance_id)));

    let icon_button = |glyph: char, color: iced::Color, m: Message| {
        button(theme::icon(glyph).size(10).color(color))
            .padding([3, 5])
            .on_press(m)
            .style(|_theme, status| theme::small_button_style(status))
    };
    let menu_open = r.ui.mixer.slot_menu == Some(instance_id);
    // ↗ — lit while the slot's window is up, and then it closes it.
    let (open_message, open_color) = open_toggle_spec(r, plugin);
    let open = icon_button(GLYPH_OPEN, open_color, open_message);
    let menu = icon_button(
        GLYPH_MENU,
        if menu_open { theme::ACCENT } else { theme::TEXT_2 },
        ui(ChainUiMessage::ToggleSlotMenu(instance_id)),
    );
    let remove = button(text("\u{00d7}").size(13).color(theme::TEXT_3))
        .padding([0, 5])
        .on_press(reorder::remove_message(owner, instance_id))
        .style(|_theme, status| theme::small_button_style(status));

    let border_color = match (focused, missing) {
        (true, _) => theme::ACCENT,
        (false, true) => theme::BAD_LINE,
        (false, false) => theme::LINE_2,
    };
    let background = if dragging { theme::BG_3 } else { theme::BG_2 };
    container(
        row![
            handle,
            bypass,
            Space::new().width(2),
            name,
            Space::new().width(4),
            open,
            menu,
            remove,
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center),
    )
    .padding([5, 6])
    .width(Length::Fill)
    .style(move |_theme| container::Style {
        background: Some(iced::Background::Color(background)),
        border: iced::Border {
            color: border_color,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    })
    .into()
}

/// What a row's `↗` carries and how it is tinted: open the slot's window,
/// or — while that window is up — close it, tinted accent so the row
/// says the window is open.
///
/// A plugin with its own GUI toggles its floating editor
/// ([`crate::view::mixer::editor_toggle_spec`], which follows the
/// engine's report, never the press). Anything else — no GUI, missing,
/// unavailable — opens the host-drawn generic window, so the toggle
/// follows that window.
pub(crate) fn open_toggle_spec(
    r: &crate::Resonance,
    plugin: &PluginSlotState,
) -> (Message, iced::Color) {
    let id = plugin.instance_id;
    if plugin.availability.reason().is_none() {
        if let Some(spec) = crate::view::mixer::editor_toggle_spec(plugin) {
            return spec;
        }
    }
    if r.ui.mixer.plugin_window_id() == Some(id) {
        (
            Message::Plugin(PluginMessage::ClosePluginWindow(id)),
            theme::ACCENT,
        )
    } else {
        (
            Message::Plugin(PluginMessage::OpenPluginWindow(id)),
            theme::TEXT_DIM,
        )
    }
}

/// The accent line marking where a dragged row will land.
fn drop_indicator() -> Element<'static, Message> {
    container(Space::new().width(Length::Fill).height(2))
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::ACCENT)),
            border: iced::Border {
                radius: 1.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// The ☰ menu's entries for one slot, in display order: the label and
/// the message the entry raises (`None` = shown disabled).
///
/// Presets need a plugin behind the slot, so a missing plugin offers
/// none of them; Move up / Move down come from [`reorder::chain_moves`],
/// so the menu and the chain rule cannot disagree.
pub(crate) fn slot_menu_entries(
    r: &crate::Resonance,
    owner: PluginOwner,
    plugin: &PluginSlotState,
    index: usize,
    len: usize,
) -> Vec<(&'static str, Option<Message>)> {
    let id = plugin.instance_id;
    let available = plugin.availability.reason().is_none();
    let preset = |m: PresetUiMessage| Message::Plugin(PluginMessage::PresetUi(m));
    let moves = reorder::chain_moves(r, owner, id, index, len);
    let mut entries: Vec<(&'static str, Option<Message>)> = vec![(
        "Parameters\u{2026}",
        available.then(|| pick(Message::Plugin(PluginMessage::OpenGenericParams(id)))),
    )];
    if available {
        entries.push((
            "Browse presets\u{2026}",
            Some(pick(ui(ChainUiMessage::BrowsePresets(id)))),
        ));
        entries.push((
            "Previous preset",
            Some(pick(preset(PresetUiMessage::Step {
                instance_id: id,
                delta: -1,
            }))),
        ));
        entries.push((
            "Next preset",
            Some(pick(preset(PresetUiMessage::Step {
                instance_id: id,
                delta: 1,
            }))),
        ));
        entries.push((
            "Save preset\u{2026}",
            Some(pick(ui(ChainUiMessage::BeginPresetSave(id)))),
        ));
    }
    entries.push(("Replace\u{2026}", Some(pick(ui(ChainUiMessage::BeginReplace(id))))));
    entries.push(("Move up", moves.up.map(pick)));
    entries.push(("Move down", moves.down.map(pick)));
    entries.push(("Remove", Some(pick(reorder::remove_message(owner, id)))));
    entries
}

/// The open ☰ menu: a small card, right-aligned under its row.
fn slot_menu(
    r: &crate::Resonance,
    owner: PluginOwner,
    plugin: &PluginSlotState,
    index: usize,
    len: usize,
) -> Element<'static, Message> {
    let mut items = column![].spacing(0);
    for (label, message) in slot_menu_entries(r, owner, plugin, index, len) {
        let enabled = message.is_some();
        let danger = label == "Remove";
        let color = match (enabled, danger) {
            (false, _) => theme::TEXT_4,
            (true, true) => theme::BAD,
            (true, false) => theme::TEXT_1,
        };
        let mut b = button(text(label).size(11).color(color))
            .width(Length::Fill)
            .padding([4, 10])
            .style(|_theme, status| theme::transport_button_style(status));
        if let Some(m) = message {
            b = b.on_press(m);
        }
        items = items.push(b);
    }
    let card = container(items)
        .width(Length::Fixed(170.0))
        .padding([4, 0])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_3)),
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        });
    container(card)
        .width(Length::Fill)
        .align_x(alignment::Horizontal::Right)
        .into()
}

/// The widget id of the "Save preset…" name field. One prompt is open
/// at a time, so one id serves every row.
pub(crate) fn preset_name_input_id() -> iced::widget::Id {
    iced::widget::Id::new("chain-preset-name")
}

/// The "Save preset…" name prompt under a row.
fn preset_save_prompt(prompt: &crate::state::SlotPresetSaveState) -> Element<'static, Message> {
    let input = text_input("Preset name", &prompt.name)
        .id(preset_name_input_id())
        .on_input(|s| ui(ChainUiMessage::PresetSaveName(s)))
        .on_submit(ui(ChainUiMessage::CommitPresetSave))
        .size(12)
        .padding([5, 8])
        .width(Length::Fill);
    let label = if prompt.exists { "Overwrite" } else { "Save" };
    let mut save = button(text(label).size(11).color(theme::TEXT_1))
        .padding([5, 8])
        .style(|_theme, status| theme::small_button_style(status));
    if !prompt.name.trim().is_empty() {
        save = save.on_press(ui(ChainUiMessage::CommitPresetSave));
    }
    let cancel = button(text("Cancel").size(11).color(theme::TEXT_2))
        .padding([5, 8])
        .on_press(ui(ChainUiMessage::CancelPresetSave))
        .style(|_theme, status| theme::small_button_style(status));
    row![input, save, cancel]
        .spacing(4)
        .align_y(alignment::Vertical::Center)
        .into()
}

/// A missing plugin's recovery, inline under its row (Q16): what is
/// missing and why, then Replace (the same `ReplacePlugin` that keeps
/// the slot's place), Remove (which discards the settings the slot is
/// keeping, as the note says), and Rescan for a plugin reinstalled
/// since the app started.
fn missing_recovery(
    r: &crate::Resonance,
    owner: PluginOwner,
    plugin: &PluginSlotState,
    reason: &str,
) -> Element<'static, Message> {
    let instance_id = plugin.instance_id;
    let candidates = replacement_candidates(r, instance_id);
    let mut body = column![
        text(format!(
            "\u{26a0} {} is not available on this machine",
            plugin.plugin_name
        ))
        .size(11)
        .color(theme::BAD),
        text(reason.to_owned()).size(10).color(theme::TEXT_2),
        text(
            "Its settings are kept with this slot: reinstall the plugin and rescan, \
             or pick a replacement. Removing the slot discards them."
        )
        .size(10)
        .color(theme::TEXT_3),
    ]
    .spacing(4);

    let mut actions = row![].spacing(4).align_y(alignment::Vertical::Center);
    if candidates.is_empty() {
        actions = actions.push(
            text("No plugins scanned")
                .size(10)
                .color(theme::TEXT_3)
                .width(Length::Fill),
        );
    } else {
        actions = actions.push(
            pick_list(candidates, None::<ScannedPlugin>, move |plugin: ScannedPlugin| {
                pick(Message::Plugin(PluginMessage::ReplacePlugin {
                    instance_id,
                    plugin,
                }))
            })
            .placeholder("Replace with\u{2026}")
            .text_size(11)
            .padding([5, 8])
            .width(Length::Fill),
        );
    }
    let small = |label: &'static str, m: Message| {
        button(text(label).size(11).color(theme::TEXT_2))
            .padding([5, 8])
            .on_press(m)
            .style(|_theme, status| theme::small_button_style(status))
    };
    actions = actions
        .push(small("Rescan", Message::Plugin(PluginMessage::RescanPlugins)))
        .push(small("Remove", reorder::remove_message(owner, instance_id)));
    body = body.push(actions);

    container(body)
        .padding([6, 8])
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BAD_DIM)),
            border: iced::Border {
                color: theme::BAD_LINE,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Replace mode: when "Replace…" was picked for a slot of this chain,
/// the group's add picker becomes a picker of replacements for it (the
/// slot keeps its place — `PluginMessage::ReplacePlugin`), with Cancel.
/// `None` when no slot of `plugins` is being replaced.
pub(super) fn replace_picker(
    r: &crate::Resonance,
    plugins: &[PluginSlotState],
) -> Option<Element<'static, Message>> {
    let instance_id = r.ui.mixer.replacing_slot?;
    let slot = plugins.iter().find(|p| p.instance_id == instance_id)?;
    let candidates = replacement_candidates(r, instance_id);
    let picker = pick_list(candidates, None::<ScannedPlugin>, move |plugin: ScannedPlugin| {
        pick(Message::Plugin(PluginMessage::ReplacePlugin {
            instance_id,
            plugin,
        }))
    })
    .placeholder(format!(
        "Replace {} with\u{2026}",
        crate::util::short(&slot.plugin_name, 18)
    ))
    .text_size(12)
    .padding([8, 10])
    .width(Length::Fill);
    let cancel = button(text("Cancel").size(11).color(theme::TEXT_2))
        .padding([8, 10])
        .on_press(ui(ChainUiMessage::CancelReplace))
        .style(|_theme, status| theme::small_button_style(status));
    Some(
        row![picker, cancel]
            .spacing(4)
            .align_y(alignment::Vertical::Center)
            .into(),
    )
}

/// Which plugins may take over a slot: instruments for a track's
/// instrument slot, effects everywhere else.
///
/// The distinction is not cosmetic — an instrument in an insert slot
/// receives no MIDI and an effect in the instrument slot leaves the
/// track with no sound source — and it is the same split the
/// `+ Add instrument` / `+ Add to chain` pickers already make.
pub(crate) fn replacement_candidates(
    r: &crate::Resonance,
    instance_id: PluginInstanceId,
) -> std::rc::Rc<[ScannedPlugin]> {
    let is_instrument_slot = r.registry.tracks.iter().any(|t| {
        crate::plugin_chain::instrument_slot(r, t)
            .and_then(|i| t.plugins.get(i))
            .is_some_and(|p| p.instance_id == instance_id)
    });
    if is_instrument_slot {
        r.ui.view_caches.instrument_plugins.clone()
    } else {
        r.ui.view_caches.fx_plugins.clone()
    }
}

/// Hash every input the CHAIN rows read beyond the plugins' identity:
/// the focus, the open menu / prompt / replace / drag, and each slot's
/// availability. The owner's fingerprint calls this so a focus change
/// or a menu opening redraws its lazy body.
pub(crate) fn hash_chain_ui<H: std::hash::Hasher>(
    h: &mut H,
    r: &crate::Resonance,
    plugins: &[PluginSlotState],
) {
    use std::hash::Hash;
    let mixer = &r.ui.mixer;
    mixer.focused_slot.hash(h);
    mixer.slot_menu.hash(h);
    mixer.replacing_slot.hash(h);
    mixer.slot_preset_save.hash(h);
    mixer.chain_drag.hash(h);
    mixer.instrument_picker_cue.hash(h);
    // ↗ follows the slot's window: its editor, or the generic window.
    mixer.plugin_window_id().hash(h);
    for p in plugins {
        p.availability.reason().hash(h);
        p.clap_plugin_id.hash(h);
        p.clap_file_path.hash(h);
        p.has_gui.hash(h);
        p.editor_open.hash(h);
    }
    // The drop indicator asks the chain rule, which reads the catalog.
    std::rc::Rc::as_ptr(&r.ui.view_caches.instrument_plugins).hash(h);
    std::rc::Rc::as_ptr(&r.ui.view_caches.fx_plugins).hash(h);
}
