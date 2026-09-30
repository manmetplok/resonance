//! The host's preset surfaces (plugin-preset-library.md §6.6, slice P6):
//! the compact bar in the plugin panel's header, the browser overlay it
//! opens over one plugin, and the media browser's Presets tab body.
//!
//! Everything drawn here is precomputed in `update::plugin_preset_ui`
//! (the lists, the pick-list choices); the one library read in `view` is
//! the loaded preset's star, a lookup in the in-memory marks snapshot.

use iced::widget::{
    button, column, container, mouse_area, opaque, pick_list, row, scrollable, stack, text,
    text_input, Space,
};
use iced::{alignment, Element, Length};

use crate::commands::CommandId;
use crate::message::*;
use crate::state::presets::{HostPresetList, HostPresetRow};
use crate::state::PluginSlotState;
use crate::theme;
use crate::view::shortcut_hint::with_hint;
use crate::Resonance;

const BROWSER_WIDTH: f32 = 520.0;
const BROWSER_LIST_HEIGHT: f32 = 320.0;

/// The star, from the icon font (the UI font has no ★): WARM when set,
/// muted otherwise — the Files tab's favourite star.
fn star_glyph<'a>(favorite: bool, size: f32) -> iced::widget::Text<'a> {
    text(String::from(theme::fa::STAR))
        .font(theme::ICON_FONT)
        .size(size)
        .color(if favorite { theme::WARM } else { theme::TEXT_3 })
}

fn ui(m: PresetUiMessage) -> Message {
    Message::Plugin(PluginMessage::PresetUi(m))
}

// ---------------------------------------------------------------------------
// Bar
// ---------------------------------------------------------------------------

/// The plugin panel's preset bar: ◀ name • ▶ ☆ Presets…, for any plugin
/// (third-party and GUI-less ones included).
pub(crate) fn preset_bar<'a>(r: &'a Resonance, plugin: &'a PluginSlotState) -> Element<'a, Message> {
    let instance_id = plugin.instance_id;
    let identity = r.presets.plugin_preset_identity.get(&instance_id);
    let name = identity.map_or("\u{2014} preset \u{2014}", |i| i.name.as_str());
    let modified = identity.is_some_and(|i| i.modified);
    let favorite = identity.is_some_and(|i| {
        crate::plugin_preset_library::library(r)
            .preset_marks(&plugin.clap_plugin_id, &i.id)
            .favorite
    });

    let small = |label: &'static str, msg: Message| {
        button(text(label).size(10).color(theme::TEXT))
            .on_press(msg)
            .style(|_theme, status| theme::small_button_style(status))
            .padding([2, 6])
    };
    let prev = with_hint(
        r,
        small("\u{25c0}", ui(PresetUiMessage::Step { instance_id, delta: -1 })),
        CommandId::PreviousPluginPreset,
    );
    let next = with_hint(
        r,
        small("\u{25b6}", ui(PresetUiMessage::Step { instance_id, delta: 1 })),
        CommandId::NextPluginPreset,
    );
    let label = row![
        text(name.to_string()).size(11).color(theme::TEXT),
        text(if modified { " \u{2022}" } else { "" })
            .size(11)
            .color(theme::TEXT_3),
    ];
    let name_button = button(label)
        .on_press(ui(PresetUiMessage::OpenBrowser(instance_id)))
        .style(|_theme, status| theme::ghost_button_style(status))
        .padding([2, 6])
        .width(Length::Fixed(180.0));
    let star = button(star_glyph(favorite, 11.0))
        .style(|_theme, status| theme::ghost_button_style(status))
        .padding([2, 4]);
    let star = if identity.is_some() {
        star.on_press(ui(PresetUiMessage::ToggleFavorite(instance_id)))
    } else {
        star
    };
    let browse = with_hint(
        r,
        small("Presets\u{2026}", ui(PresetUiMessage::OpenBrowser(instance_id))),
        CommandId::BrowsePluginPresets,
    );
    row![prev, name_button, next, star, browse]
        .spacing(4)
        .align_y(alignment::Vertical::Center)
        .into()
}

// ---------------------------------------------------------------------------
// List (shared by the overlay and the media tab)
// ---------------------------------------------------------------------------

fn preset_row<'a>(
    row_data: &'a HostPresetRow,
    selected: bool,
    show_plugin: bool,
    on_click: Message,
    on_double: Message,
    on_star: Message,
) -> Element<'a, Message> {
    let mut label = column![text(row_data.name.clone()).size(12).color(theme::TEXT)].spacing(1);
    let mut sub = Vec::new();
    if show_plugin {
        sub.push(row_data.plugin_name.clone());
    }
    if let Some(c) = &row_data.category {
        sub.push(c.clone());
    }
    if row_data.source == resonance_control::methods::plugin_preset::PluginPresetSource::User {
        sub.push("user".to_string());
    }
    if !sub.is_empty() {
        label = label.push(text(sub.join(" \u{00b7} ")).size(10).color(theme::TEXT_3));
    }
    let star = button(star_glyph(row_data.favorite, 12.0))
        .on_press(on_star)
        .style(|_theme, status| theme::ghost_button_style(status))
        .padding([2, 4]);
    // A container, not a button: a button captures the press, and the
    // mouse area's double-click would then never see it.
    let body = container(label)
        .width(Length::Fill)
        .padding([4, 8])
        .style(move |_theme| row_style(selected));
    let body = mouse_area(body)
        .on_press(on_click)
        .on_double_click(on_double)
        .interaction(iced::mouse::Interaction::Pointer);
    row![body, star]
        .spacing(4)
        .align_y(alignment::Vertical::Center)
        .into()
}

/// A preset row's card: the Files tab's row look, WARM when selected.
fn row_style(selected: bool) -> container::Style {
    let (bg, border) = if selected {
        (theme::WARM_DIM, theme::WARM_LINE)
    } else {
        (theme::BG_3, theme::LINE_2)
    };
    container::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: Some(theme::TEXT_1),
        border: iced::Border {
            color: border,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

fn search_row<'a>(
    list: &'a HostPresetList,
    on_input: fn(String) -> Message,
    on_favorites: fn(bool) -> Message,
) -> Element<'a, Message> {
    let only = list.favorites_only;
    row![
        text_input("Search presets\u{2026}", &list.query)
            .on_input(on_input)
            .size(12)
            .padding([4, 6])
            .style(theme::borderless_text_input_style),
        button(
            row![
                star_glyph(only, 10.0),
                text("only")
                    .size(10)
                    .color(if only { theme::WARM } else { theme::TEXT_2 }),
            ]
            .spacing(4)
            .align_y(alignment::Vertical::Center),
        )
        .on_press(on_favorites(!only))
        .style(move |_theme, status| theme::tab_button_style(only, status))
        .padding([3, 8]),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center)
    .into()
}

// ---------------------------------------------------------------------------
// Overlay
// ---------------------------------------------------------------------------

/// The browser over one plugin: search, ★ only, the list (click to
/// audition, double-click or Load to keep), Revert. Esc reverts; a click on
/// the backdrop keeps.
pub(crate) fn view_preset_browser_overlay(r: &Resonance) -> Element<'_, Message> {
    let Some(browser) = r.presets.host_browser.as_ref() else {
        return Space::new().into();
    };
    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.4,
                ))),
                ..Default::default()
            }),
    )
    .on_press(ui(PresetUiMessage::CloseBrowser { keep: true }));

    let header = row![
        text("PRESETS")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        text(browser.plugin_name.clone()).size(12).color(theme::ACCENT),
        Space::new().width(Length::Fill),
        button(text("\u{00d7}").size(14).color(theme::TEXT_DIM))
            .on_press(ui(PresetUiMessage::CloseBrowser { keep: false }))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let list = &browser.list;
    let mut rows = column![].spacing(2);
    if list.rows.is_empty() {
        rows = rows.push(text("No presets match").size(12).color(theme::TEXT_3));
    }
    for (i, row_data) in list.rows.iter().enumerate() {
        rows = rows.push(preset_row(
            row_data,
            list.selected == Some(i),
            false,
            ui(PresetUiMessage::BrowserAudition(i)),
            ui(PresetUiMessage::CloseBrowser { keep: true }),
            ui(PresetUiMessage::BrowserToggleRowFavorite(i)),
        ));
    }

    let auditioning = browser.origin.is_some();
    let hint = if auditioning {
        "Auditioning \u{2014} Load keeps it, Esc reverts"
    } else {
        "Click a preset to audition it"
    };
    let load = button(text("Load").size(11))
        .style(|_theme, status| theme::primary_button_style(status))
        .padding([4, 12]);
    let load = if auditioning {
        load.on_press(ui(PresetUiMessage::CloseBrowser { keep: true }))
    } else {
        load
    };
    let footer = row![
        text(hint).size(10).color(theme::TEXT_3),
        Space::new().width(Length::Fill),
        button(text("Revert").size(11).color(theme::TEXT))
            .on_press(ui(PresetUiMessage::CloseBrowser { keep: false }))
            .style(|_theme, status| theme::small_button_style(status))
            .padding([4, 12]),
        load,
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let panel = container(opaque(
        column![
            header,
            search_row(
                list,
                |s| ui(PresetUiMessage::BrowserSearch(s)),
                |on| ui(PresetUiMessage::BrowserFavoritesOnly(on)),
            ),
            scrollable(rows).height(Length::Fixed(BROWSER_LIST_HEIGHT)),
            footer,
        ]
        .spacing(10)
        .padding(14),
    ))
    .width(BROWSER_WIDTH)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_LG.into(),
        },
        ..Default::default()
    });

    let centered = container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center);
    stack![backdrop, centered].into()
}

// ---------------------------------------------------------------------------
// Media browser tab
// ---------------------------------------------------------------------------

/// The Presets tab: the library across every plugin, a plugin filter, ★
/// only. Double-click loads onto the selected plugin slot.
pub(crate) fn media_presets_body(r: &Resonance) -> Element<'_, Message> {
    let list = &r.presets.media_presets;
    let current = r
        .presets
        .media_plugin_choices
        .iter()
        .find(|c| c.plugin_id == list.plugin)
        .cloned();
    let plugin_picker = pick_list(
        r.presets.media_plugin_choices.clone(),
        current,
        |choice| ui(PresetUiMessage::MediaPlugin(choice)),
    )
    .placeholder("All plugins")
    .text_size(11)
    .padding([4, 8])
    .width(Length::Fill);

    let mut rows = column![].spacing(2);
    if list.rows.is_empty() {
        rows = rows.push(text("No presets match").size(12).color(theme::TEXT_3));
    }
    for (i, row_data) in list.rows.iter().enumerate() {
        rows = rows.push(preset_row(
            row_data,
            list.selected == Some(i),
            list.plugin.is_none(),
            ui(PresetUiMessage::MediaPress(i)),
            ui(PresetUiMessage::MediaLoad(i)),
            ui(PresetUiMessage::MediaToggleRowFavorite(i)),
        ));
    }
    let target = r
        .ui
        .mixer
        .selected_plugin
        .and_then(|id| r.plugin_slot(id))
        .map(|slot| format!("Double-click loads onto {}", slot.plugin_name))
        .unwrap_or_else(|| "Select a plugin slot, then double-click a preset".to_string());

    column![
        search_row(
            list,
            |s| ui(PresetUiMessage::MediaSearch(s)),
            |on| ui(PresetUiMessage::MediaFavoritesOnly(on)),
        ),
        plugin_picker,
        scrollable(rows).height(Length::Fill),
        text(target).size(10).color(theme::TEXT_3),
    ]
    .spacing(8)
    .height(Length::Fill)
    .into()
}
