//! Export modal — shared shell (design doc #155, todo #324).
//!
//! One overlay with two mode tabs (Audio stems / MIDI) and a shared
//! footer. This file builds ONLY the scaffold: the dimmed backdrop, the
//! centered container, the serif-italic title, the mode tabs, and the
//! footer (live count + primary action). The per-tab body widgets — the
//! source checklist, range/format controls, destination — land in todos
//! #326 (stems) and #327 (MIDI); the progress/done/error phases land in
//! #328. Until then each tab shows a placeholder region.
//!
//! Same backdrop + centered-dialog pattern as `bounce_dialog.rs`.
use iced::widget::{button, column, container, mouse_area, opaque, row, stack, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::state::{ExportMode, ExportPhase};
use crate::theme;
use crate::Resonance;

pub(crate) fn view_export_dialog_overlay<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let Some(dialog) = r.export_dialog.as_ref() else {
        return Space::new().width(Length::Fixed(0.0)).height(Length::Fixed(0.0)).into();
    };

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
    .on_press(Message::Export(ExportMessage::Close));

    let title = text("Export")
        .size(20)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1);

    // Mode tabs — segmented toggle mirroring the bounce dialog's
    // stereo/mono buttons. The active tab gets the accent border so the
    // current mode reads at a glance.
    let tab = |label: &'static str, mode: ExportMode| {
        let selected = dialog.mode == mode;
        let mut b = button(
            text(label)
                .size(13)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(if selected { iced::Color::WHITE } else { theme::TEXT_2 }),
        )
        .padding([6, 16])
        .style(move |_t, status| {
            let mut s = theme::transport_button_style(status);
            if selected {
                s.border.color = theme::ACCENT;
                s.background = Some(iced::Background::Color(theme::ACCENT_DIM));
            }
            s
        });
        if !selected {
            b = b.on_press(Message::Export(ExportMessage::SetMode(mode)));
        }
        b
    };
    let tabs = row![
        tab("Audio stems", ExportMode::AudioStems),
        tab("MIDI", ExportMode::Midi),
    ]
    .spacing(8);

    // Per-tab body. The real controls land in #326/#327 — for now a
    // placeholder region keyed off the active mode so the shell is
    // navigable and snapshot-testable.
    let body_hint = match dialog.mode {
        ExportMode::AudioStems => {
            "Sources, render range & format, and destination land here (todo #326)."
        }
        ExportMode::Midi => {
            "Filtered sources, file layout, tempo embed, and destination land here (todo #327)."
        }
    };
    let body = container(text(body_hint).size(12).color(theme::TEXT_3))
        .width(Length::Fill)
        .height(Length::Fixed(200.0))
        .padding(16)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::LINE_2)),
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        });

    // Footer: live count on the left, Cancel + primary action on the
    // right. The primary label and count noun depend on the mode.
    let count = dialog.selected_count();
    let (count_label, action_label) = match dialog.mode {
        ExportMode::AudioStems => (
            format!("{count} selected"),
            if count == 0 {
                "Export stems".to_string()
            } else {
                format!("Export {count} stems")
            },
        ),
        ExportMode::Midi => (
            format!("{count} selected"),
            if count == 0 {
                "Export MIDI".to_string()
            } else {
                format!("Export {count} MIDI files")
            },
        ),
    };

    let count_text = text(count_label).size(12).color(theme::TEXT_2);

    let cancel_btn = button(text("Cancel").size(13).color(theme::TEXT_1))
        .on_press(Message::Export(ExportMessage::Close))
        .padding([8, 18])
        .style(|_theme, status| theme::ghost_button_style(status));

    let can_export = dialog.can_export();
    let mut action_btn = button(
        text(action_label)
            .size(13)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(if can_export { theme::BG_0 } else { theme::TEXT_3 }),
    )
    .padding([8, 18])
    .style(move |_theme, status| {
        if can_export {
            theme::primary_button_style(status)
        } else {
            theme::ghost_button_style(status)
        }
    });
    if can_export {
        action_btn = action_btn.on_press(Message::Export(ExportMessage::Confirm));
    }

    let footer = row![
        count_text,
        Space::new().width(Length::Fill),
        cancel_btn,
        action_btn,
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let dialog_content = column![
        title,
        Space::new().height(14),
        tabs,
        Space::new().height(14),
        body,
        Space::new().height(20),
        footer,
    ]
    .spacing(0)
    .padding(24)
    .width(560);

    let dialog_box = container(dialog_content).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: theme::RADIUS_XL.into(),
        },
        ..Default::default()
    });

    let centered = container(opaque(dialog_box))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    // The shell only ever shows the `Setup` phase; the render phases
    // (Rendering / Done / Error) are drawn by todo #328.
    debug_assert!(matches!(dialog.phase, ExportPhase::Setup));

    stack![backdrop, centered].into()
}
