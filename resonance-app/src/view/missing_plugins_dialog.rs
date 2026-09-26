//! Missing-plugin **load warning** (ba doc #275 P5, todo #1309).
//!
//! Raised when a project is opened on a machine that hasn't got one of
//! its plugins. Before this existed, the load reported a generic
//! `Failed to load plugin: …` toast that named no track and no slot —
//! and if several plugins were missing, five toasts arrived and the user
//! saw the last one. The chain kept a placeholder for each, looking
//! exactly like a working plugin and doing nothing.
//!
//! So this modal answers the two questions that toast could not: *which*
//! plugins, and *where* they sit. One row per dead slot, each naming the
//! plugin, the chain it is in (track, bus or master) and its position, so
//! the user can go straight to it.
//!
//! It deliberately carries **no per-row action**. The recovery gestures —
//! replace, relocate, remove — live on the slot itself (the mixer strip's
//! warning pill opens the plugin panel, which holds the picker), because
//! a plugin is a position in a chain and choosing its replacement is a
//! decision made looking at that chain, not at a list. What the modal
//! offers instead is the one action that can fix *every* row at once:
//! install the plugins and **Rescan** (ba todo #1307).
//!
//! Shares the relink modal's scaffold — dimmed backdrop, centered `BG_2`
//! card, serif-italic title, `BAD` for the error state — because it is
//! the same kind of news about a different kind of missing thing.

use iced::widget::{
    button, column, container, mouse_area, opaque, row, scrollable, stack, text, Space,
};
use iced::{alignment, Element, Length};

use crate::message::{Message, PluginMessage, UiMessage};
use crate::state::{MissingPluginSlot, PluginLocator};
use crate::theme;
use crate::Resonance;

/// Fixed width of the modal card. Wider than the relink modal's 520 px:
/// a row here carries a plugin name AND the chain position it sits at.
const CARD_WIDTH: f32 = 560.0;

/// Cap on the scrollable list height so a project missing a dozen
/// plugins still leaves the header, note and footer on screen.
const LIST_MAX_HEIGHT: f32 = 260.0;

/// The missing-plugin warning overlay. Returns a zero-sized element when
/// it isn't open or has nothing to report, so the caller can stack it
/// unconditionally.
pub(crate) fn view_missing_plugins_overlay(r: &Resonance) -> Element<'_, Message> {
    let rows = r.missing_plugin_slots();
    if !r.missing_plugins.modal_open || rows.is_empty() {
        return Space::new()
            .width(Length::Fixed(0.0))
            .height(Length::Fixed(0.0))
            .into();
    }

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.6,
                ))),
                ..Default::default()
            }),
    )
    .on_press(Message::Ui(UiMessage::DismissMissingPlugins));

    let title = text(headline(rows.len()))
        .size(20)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1);

    let subtitle = text(
        "This project uses plugins that aren't installed here. Their slots are \
         kept in place, holding the settings you saved, so installing the \
         plugins and rescanning brings the sound back exactly as it was. \
         Nothing is lost until you remove a slot.",
    )
    .size(12)
    .color(theme::TEXT_2);

    let mut list = column![].spacing(6);
    for slot in &rows {
        list = list.push(missing_row(slot));
    }
    let list = container(scrollable(list).width(Length::Fill)).max_height(LIST_MAX_HEIGHT);

    let note = row![
        theme::icon(theme::fa::CIRCLE_INFO)
            .size(12)
            .color(theme::TEXT_3),
        text(
            "To swap one out instead, click its slot in the mixer: the plugin \
             panel offers a replacement and keeps the chain position.",
        )
        .size(11)
        .color(theme::TEXT_3),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let body = column![
        subtitle,
        Space::new().height(12),
        list,
        Space::new().height(12),
        note
    ]
    .spacing(0);

    let dismiss_btn = button(text("Continue anyway").size(13).color(theme::TEXT_1))
        .on_press(Message::Ui(UiMessage::DismissMissingPlugins))
        .padding([8, 18])
        .style(|_theme, status| theme::ghost_button_style(status));

    let rescan_label = if r.plugin_catalog.plugin_scan_in_progress {
        "Scanning\u{2026}"
    } else {
        "Rescan plugins"
    };
    let mut rescan_btn = button(
        row![
            theme::icon(theme::fa::ARROW_ROTATE_LEFT).size(12),
            text(rescan_label).size(13),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center),
    )
    .padding([8, 18])
    .style(|_theme, status| theme::primary_button_style(status));
    if !r.plugin_catalog.plugin_scan_in_progress {
        rescan_btn = rescan_btn.on_press(Message::Plugin(PluginMessage::RescanPlugins));
    }

    let footer = row![
        text(format!("{} affected", slot_count_label(rows.len())))
            .size(12)
            .color(theme::TEXT_2),
        Space::new().width(Length::Fill),
        dismiss_btn,
        rescan_btn,
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let card_content = column![
        title,
        Space::new().height(8),
        body,
        Space::new().height(20),
        footer,
    ]
    .spacing(0)
    .padding(24)
    .width(CARD_WIDTH);

    let card = container(card_content).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: theme::RADIUS_XL.into(),
        },
        ..Default::default()
    });

    let centered = container(opaque(card))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    stack![backdrop, centered].into()
}

/// One dead slot: warning glyph, the plugin's name + the identity the
/// project recorded, and where in the mix it sits.
fn missing_row(slot: &MissingPluginSlot) -> Element<'static, Message> {
    let name_col = column![
        text(slot.plugin_name.clone()).size(12).color(theme::TEXT_1),
        text(slot.clap_plugin_id.clone())
            .size(10)
            .color(theme::TEXT_4),
    ]
    .spacing(2)
    .width(Length::Fill);

    let inner = row![
        theme::icon(theme::fa::TRIANGLE_EXCLAMATION)
            .size(13)
            .color(theme::BAD),
        name_col,
        text(slot_location(slot)).size(11).color(theme::TEXT_2),
    ]
    .spacing(10)
    .align_y(alignment::Vertical::Center)
    .padding([8, 12]);

    container(inner)
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Where a dead slot sits, as the user would describe it: the chain's
/// name and the 1-based position in it.
///
/// 1-based because this is prose for a person, not an address for the
/// control API — `plugin_params` reports the same slot 0-based, and the
/// two audiences are different.
pub(crate) fn slot_location(slot: &MissingPluginSlot) -> String {
    let kind = match slot.owner {
        PluginLocator::Track(_) => "track",
        PluginLocator::Bus(_) => "bus",
        PluginLocator::Master => "chain",
    };
    format!(
        "{} {} \u{00b7} slot {}",
        slot.owner_label,
        kind,
        slot.slot + 1
    )
}

/// The modal headline, singular/plural aware.
fn headline(count: usize) -> String {
    if count == 1 {
        "1 plugin couldn't be loaded".to_string()
    } else {
        format!("{count} plugins couldn't be loaded")
    }
}

fn slot_count_label(count: usize) -> String {
    if count == 1 {
        "1 slot".to_string()
    } else {
        format!("{count} slots")
    }
}
