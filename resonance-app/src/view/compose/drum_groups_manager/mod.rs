//! Drum Groups Manager modal — three-column layout for building drum
//! groups out of kit pads:
//!
//!   1. Groups list (left, 240 px).
//!   2. Active group detail (middle).
//!   3. Kit pad picker (right) with filter + categories.

use iced::widget::{
    button, column, container, mouse_area, opaque, row, stack, text, Space,
};
use iced::{alignment, Color, Element, Length};

use crate::compose::messages::DrumGroupsMessage;
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::theme;
use crate::Resonance;

mod detail;
mod groups_list;
mod kit_picker;

use detail::group_detail_column;
use groups_list::groups_list_column;
use kit_picker::kit_picker_column;

pub fn view<'a>(r: &'a Resonance) -> Element<'a, Message> {
    if !r.compose.drumroll.manager_open {
        return container(Space::new().height(0)).into();
    }

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.7,
                ))),
                ..Default::default()
            }),
    )
    .on_press(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::CloseManager,
    )));

    let header = modal_header();
    let body = three_column_body(r);

    let sheet = container(column![header, body].spacing(0))
        .width(Length::Fixed(1100.0))
        .height(Length::Fixed(660.0))
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_XL.into(),
            },
            ..Default::default()
        });

    let centered = container(opaque(sheet))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    stack![backdrop, centered].into()
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

fn modal_header<'a>() -> Element<'a, Message> {
    let title = column![
        text("DRUMS")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        text("Manage drum groups")
            .size(22)
            .font(theme::SERIF_ITALIC_FONT)
            .color(theme::TEXT_1),
        text(
            "A drum group shares one generated rhythm. Hits are distributed across the group's pads by the articulation mix — ideal for grouping hi-hat closed/open/half-open so a single 16th-note pattern moves through them naturally."
        )
        .size(11)
        .color(theme::TEXT_3),
    ]
    .spacing(2);

    let cancel = button(text("Cancel").size(12).color(theme::TEXT_2))
        .padding([8, 14])
        .style(|_theme, status| theme::ghost_button_style(status))
        .on_press(Message::Compose(ComposeMessage::DrumGroups(
            DrumGroupsMessage::CloseManager,
        )));
    let done = button(
        text("Done")
            .size(12)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(Color::from_rgb(0.10, 0.08, 0.04)),
    )
    .padding([8, 16])
    .style(|_theme, status| {
        let bg = match status {
            button::Status::Hovered => Color {
                r: (theme::WARM.r * 1.08).min(1.0),
                g: (theme::WARM.g * 1.08).min(1.0),
                b: (theme::WARM.b * 1.08).min(1.0),
                a: 1.0,
            },
            button::Status::Pressed => Color {
                r: theme::WARM.r * 0.85,
                g: theme::WARM.g * 0.85,
                b: theme::WARM.b * 0.85,
                a: 1.0,
            },
            _ => theme::WARM,
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: Color::from_rgb(0.10, 0.08, 0.04),
            border: iced::Border {
                color: theme::WARM,
                width: 0.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        }
    })
    .on_press(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::CloseManager,
    )));

    container(
        row![title, Space::new().width(Length::Fill), cancel, done]
            .spacing(8)
            .align_y(alignment::Vertical::Top),
    )
    .padding([16, 20])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_1)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    })
    .into()
}

// ---------------------------------------------------------------------------
// Body — three columns
// ---------------------------------------------------------------------------

fn three_column_body<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let groups_column = groups_list_column(r);
    let detail_column = group_detail_column(r);
    let kit_column = kit_picker_column(r);

    row![groups_column, detail_column, kit_column]
        .spacing(0)
        .height(Length::Fill)
        .into()
}

pub(super) fn col_head<'a>(title: &str, meta: String) -> Element<'a, Message> {
    let owned_title = title.to_string();
    container(
        row![
            text(owned_title)
                .size(10)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::TEXT_3),
            Space::new().width(Length::Fill),
            text(meta).size(10).font(theme::MONO_FONT).color(theme::TEXT_3),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .padding([8, 0])
    .width(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_1)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    })
    .into()
}

pub(super) fn separator_below<'a>() -> Element<'a, Message> {
    container(Space::new().height(1))
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::LINE_2)),
            ..Default::default()
        })
        .into()
}

pub(super) fn column_panel<'a>(content: Element<'a, Message>, width: Length) -> Element<'a, Message> {
    container(content)
        .padding([14, 16])
        .width(width)
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

pub(super) fn empty_hint<'a>(msg: &str) -> Element<'a, Message> {
    let msg = msg.to_string();
    container(text(msg).size(11).color(theme::TEXT_3))
        .padding([16, 14])
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(Color::TRANSPARENT)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_LG.into(),
            },
            ..Default::default()
        })
        .into()
}

pub(super) fn u8_color(rgb: [u8; 3]) -> Color {
    Color::from_rgb(
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
    )
}
