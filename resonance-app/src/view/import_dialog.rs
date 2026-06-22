//! MIDI Import modal shell. Shares the export/bounce modal scaffold —
//! dimmed `mouse_area` backdrop, centered container on `BG_2` with a
//! `LINE` border and `RADIUS_XL` corners, serif-italic title — so it
//! reads as one family with `view::bounce_dialog`.
//!
//! Only the shell is built here; the per-stage bodies (drop target, the
//! review track table, the tempo-conflict and placement controls) land in
//! the follow-up view todos (doc #158). For now the body is a single
//! placeholder line keyed off the current stage so the shell is legible.

use iced::widget::{button, column, container, mouse_area, opaque, row, stack, text, Space};
use iced::{alignment, Element, Length};

use crate::message::{ImportMessage, Message};
use crate::state::ImportStage;
use crate::theme;
use crate::Resonance;

pub(crate) fn view_import_dialog_overlay<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let Some(dialog) = r.import_dialog.as_ref() else {
        return Space::new()
            .width(Length::Fixed(0.0))
            .height(Length::Fixed(0.0))
            .into();
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
    .on_press(Message::Import(ImportMessage::Cancel));

    let title = text("Import MIDI")
        .size(20)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1);

    // Per-stage bodies arrive in a follow-up todo; until then name the
    // current step so the shell isn't blank. An error message, when set,
    // takes precedence over the stage label.
    let stage_label = match dialog.stage {
        ImportStage::Drop => "Drop a MIDI file to import, or choose one.",
        ImportStage::Parsing => "Parsing\u{2026}",
        ImportStage::Review => "Review the tracks to import.",
        ImportStage::TempoConflict => "Resolve the tempo difference.",
        ImportStage::Error => "Import failed.",
        ImportStage::Imported => "Import complete.",
    };
    let body = text(dialog.error.as_deref().unwrap_or(stage_label))
        .size(13)
        .color(theme::TEXT_2);

    let cancel_btn = button(text("Cancel").size(13).color(theme::TEXT_1))
        .on_press(Message::Import(ImportMessage::Cancel))
        .padding([8, 18])
        .style(|_theme, status| theme::ghost_button_style(status));

    let button_row = row![Space::new().width(Length::Fill), cancel_btn]
        .spacing(8)
        .align_y(alignment::Vertical::Center);

    let dialog_content = column![
        title,
        Space::new().height(10),
        body,
        Space::new().height(20),
        button_row,
    ]
    .spacing(4)
    .padding(24)
    .width(460);

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

    stack![backdrop, centered].into()
}
