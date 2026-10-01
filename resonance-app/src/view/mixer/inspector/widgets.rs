//! Low-level widget helpers shared across inspector sections: toggle
//! and action buttons, labelled field wrappers, placeholder pickers and
//! rows, the title block, and the collapsible group header.

use iced::widget::{button, column, container, row, text, Space};
use iced::{alignment, Element, Length};

use crate::message::{Message, UiMessage};
use crate::state::MixerInspectorGroup;
use crate::theme;
use crate::view::controls::collapse_caret;

/// Fixed width of the right-hand "aux" column in a two-column external
/// field (the MIDI channel / return-port picker) — the prototype's
/// `grid-template-columns: 1fr 92px`.
pub(super) const EXT_AUX_COL: f32 = 92.0;

/// A two-state toggle button: neutral when off, tinted with `on_color`
/// (text/border) over `on_bg` (fill) when on.
pub(super) fn toggle_button(
    label: &'static str,
    on: bool,
    on_color: iced::Color,
    on_bg: iced::Color,
    msg: Message,
) -> Element<'static, Message> {
    button(
        text(label)
            .size(11)
            .align_x(alignment::Horizontal::Center)
            .width(Length::Fill),
    )
    .padding([7, 0])
    .width(Length::Fill)
    .on_press(msg)
    .style(move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered);
        let (bg, border, txt) = if on {
            (on_bg, on_color, on_color)
        } else if hovered {
            (theme::BG_3, theme::LINE, theme::TEXT_1)
        } else {
            (theme::BG_2, theme::LINE, theme::TEXT_3)
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: txt,
            border: iced::Border {
                color: border,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        }
    })
    .into()
}

/// Full-width hairline action button — the inspector's shape for a
/// one-shot command (Bounce, Delete bus, External hardware instrument).
/// `None` renders it disabled (dim text, no hover). `danger` tints the
/// hover toward BAD for destructive actions.
pub(super) fn action_button(
    label: &'static str,
    msg: Option<Message>,
    danger: bool,
) -> Element<'static, Message> {
    let enabled = msg.is_some();
    let mut b = button(
        text(label)
            .size(11)
            .align_x(alignment::Horizontal::Center)
            .width(Length::Fill),
    )
    .padding([7, 0])
    .width(Length::Fill)
    .style(move |_theme, status| {
        let hovered = enabled && matches!(status, button::Status::Hovered);
        let (bg, border, txt) = match (enabled, hovered, danger) {
            (false, _, _) => (theme::BG_2, theme::LINE_2, theme::TEXT_4),
            (true, true, true) => (theme::BAD_DIM, theme::BAD_LINE, theme::BAD),
            (true, true, false) => (theme::BG_3, theme::ACCENT_LINE, theme::TEXT_1),
            (true, false, _) => (theme::BG_2, theme::LINE, theme::TEXT_2),
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: txt,
            border: iced::Border {
                color: border,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        }
    });
    if let Some(msg) = msg {
        b = b.on_press(msg);
    }
    b.into()
}

/// Small neutral pill beside the header name naming the channel type
/// ("Inst", "Audio", "Bus", "Master").
pub(super) fn type_tag(label: &'static str) -> Element<'static, Message> {
    container(
        text(label)
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
    )
    .padding([1, 6])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    })
    .into()
}

/// A dim uppercase label heading a section *inside* a group (the
/// external-hardware section of TRACK). Not collapsible on its own.
pub(super) fn sub_label(title: &'static str) -> Element<'static, Message> {
    text(title)
        .size(9)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3)
        .into()
}

/// The dashed-looking placeholder row a group shows when it has nothing
/// to list ("Empty chain", "No automation lanes").
pub(super) fn placeholder_row(label: &'static str) -> Element<'static, Message> {
    container(text(label).size(11).color(theme::TEXT_3))
        .padding([8, 10])
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        })
        .into()
}

/// The inspector title block: the dim "INSPECTOR" caption over the
/// owner's title row.
pub(super) fn header<'a>(
    title_row: iced::widget::Row<'a, Message>,
) -> Element<'a, Message> {
    column![
        text("INSPECTOR")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        Space::new().height(2),
        title_row,
    ]
    .spacing(0)
    .into()
}

/// Stacked field label + picker block used inside the ROUTING group.
pub(super) fn field(
    label: &'static str,
    picker: Element<'static, Message>,
) -> Element<'static, Message> {
    column![
        text(label)
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        Space::new().height(4),
        picker,
    ]
    .spacing(0)
    .into()
}

/// Two-column field: a label (with an optional BAD-pink "offline" tag)
/// above a row of two pickers (`device` on the left grows, `aux` on the
/// right is a fixed-narrow column), matching the prototype's `.two`
/// grid (`1fr 92px`).
pub(super) fn field2(
    label: &'static str,
    offline: bool,
    device: Element<'static, Message>,
    aux: Element<'static, Message>,
) -> Element<'static, Message> {
    let mut label_row = row![text(label)
        .size(9)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3)]
    .align_y(alignment::Vertical::Center);
    if offline {
        label_row = label_row
            .push(Space::new().width(6))
            .push(text("offline").size(8).color(theme::BAD));
    }
    column![
        label_row,
        Space::new().height(4),
        row![
            container(device).width(Length::Fill),
            Space::new().width(8),
            container(aux).width(Length::Fixed(EXT_AUX_COL)),
        ]
        .align_y(alignment::Vertical::Center),
    ]
    .spacing(0)
    .into()
}

/// A non-interactive picker-shaped placeholder used where a second
/// two-column field has no live control yet (e.g. no return channels).
pub(super) fn placeholder_pick(value: &'static str) -> Element<'static, Message> {
    container(text(value).size(12).color(theme::TEXT_4))
        .width(Length::Fill)
        .padding([5, 8])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Read-only label/value row with a hairline under it — the shape the
/// ROUTING group uses for facts the user cannot edit here (a track's
/// Send A/B placeholders, a bus's members / sends / output). `muted`
/// dims the value for an empty one.
pub(super) fn info_row(
    label: &'static str,
    value: String,
    muted: bool,
) -> Element<'static, Message> {
    let value_color = if muted { theme::TEXT_4 } else { theme::TEXT_1 };
    // The value takes the remaining width and wraps inside it rather
    // than running past the pane's right edge — a bus's member list is
    // as long as the number of tracks routed into it, and clipping it
    // silently drops exactly the names the row exists to show.
    let r_row = row![
        text(label).size(11).color(theme::TEXT_3),
        Space::new().width(10),
        text(value)
            .size(12)
            .font(theme::MONO_FONT)
            .color(value_color)
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Right),
    ]
    .align_y(alignment::Vertical::Center)
    .padding([6, 0]);

    column![
        r_row,
        container(Space::new().width(Length::Fill))
            .height(1)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::LINE_2)),
                ..Default::default()
            }),
    ]
    .spacing(0)
    .into()
}

/// A small recovery-action button used inside the offline device alert
/// (matching the prototype's `.alert .fix button`).
pub(super) fn alert_action_button(label: &'static str, msg: Message) -> Element<'static, Message> {
    button(text(label).size(10).color(theme::TEXT_2))
        .padding([4, 10])
        .on_press(msg)
        .style(|_theme, status| {
            let hovered = matches!(status, button::Status::Hovered);
            button::Style {
                background: Some(iced::Background::Color(if hovered {
                    theme::BG_2
                } else {
                    theme::BG_1
                })),
                text_color: theme::TEXT_2,
                border: iced::Border {
                    color: theme::LINE,
                    width: 1.0,
                    radius: theme::RADIUS_XS.into(),
                },
                ..Default::default()
            }
        })
        .into()
}

/// Compact zero-padded bank/program numeric readout tile (e.g. `031` /
/// `012`), mirroring the prototype's `.pgnum`.
pub(super) fn pgnum_tile(value: String) -> Element<'static, Message> {
    container(
        text(value)
            .size(15)
            .font(theme::MONO_FONT)
            .color(theme::ACCENT_SOFT),
    )
    .width(Length::Fill)
    .padding([6, 0])
    .align_x(iced::alignment::Horizontal::Center)
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

/// The "<model> preset →" chip beside the PATCH label (epic #40, doc #201
/// §5). When a device preset is selected (`Some(label)`) the chip lights
/// with the accent color + border and names the device; with no selection it
/// reads as a dim hint ("Select device →").
pub(super) fn preset_hint_chip(selected_label: Option<String>) -> Element<'static, Message> {
    let active = selected_label.is_some();
    let label = match &selected_label {
        Some(name) => format!("{} \u{2192}", name),
        None => "Select device \u{2192}".to_string(),
    };
    let (text_color, bg, border) = if active {
        (theme::ACCENT_SOFT, theme::BG_1, theme::ACCENT_LINE)
    } else {
        (theme::TEXT_4, theme::BG_2, theme::LINE_2)
    };
    container(
        text(label)
            .size(8)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(text_color),
    )
    .padding([1, 5])
    .style(move |_theme| container::Style {
        background: Some(iced::Background::Color(bg)),
        border: iced::Border {
            color: border,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    })
    .into()
}

/// Clickable collapse row (title left, caret right) with a hairline
/// below. Clicking anywhere on the row folds / unfolds the group.
pub(super) fn group_header(
    title: &'static str,
    group: MixerInspectorGroup,
    collapsed: bool,
) -> Element<'static, Message> {
    let head = iced::widget::button(
        row![
            text(title)
                .size(10)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::TEXT_3),
            Space::new().width(Length::Fill),
            collapse_caret(!collapsed),
        ]
        .align_y(alignment::Vertical::Center)
        .width(Length::Fill),
    )
    .padding([2, 0])
    .width(Length::Fill)
    .style(|_theme, status| theme::small_button_style(status))
    .on_press(Message::Ui(UiMessage::ToggleMixerInspectorGroup(group)));

    column![
        head,
        Space::new().height(4),
        container(Space::new().width(Length::Fill))
            .height(1)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::LINE_2)),
                ..Default::default()
            }),
    ]
    .spacing(0)
    .into()
}
