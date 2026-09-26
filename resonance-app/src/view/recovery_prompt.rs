//! Autosave-recovery prompt (code review FU-M12a): shown when a project
//! being opened — or, at startup, a never-saved session — holds an
//! autosave that an unclean exit left behind. Same backdrop +
//! centered-dialog pattern as the quit confirmation.
use iced::widget::{button, column, container, mouse_area, opaque, row, stack, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::state::RecoveryPrompt;
use crate::theme;

/// "12 minutes" / "1 hour" style span between two instants.
fn span_words(secs: u64) -> String {
    let (n, unit) = match secs {
        0..=59 => (secs, "second"),
        60..=3_599 => (secs / 60, "minute"),
        3_600..=86_399 => (secs / 3_600, "hour"),
        _ => (secs / 86_400, "day"),
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

fn body_text(prompt: &RecoveryPrompt) -> String {
    let offer = &prompt.offer;
    if prompt.untitled {
        return "Resonance closed before an untitled project was saved. Its last autosave \
                can be recovered."
            .to_owned();
    }
    let name = offer
        .dir
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| offer.dir.display().to_string());
    let newer = offer
        .saved_at
        .and_then(|saved| offer.autosave_at.duration_since(saved).ok())
        .map(|d| format!(", {} newer than the last save", span_words(d.as_secs())))
        .unwrap_or_default();
    format!(
        "Resonance didn't close cleanly while \u{201c}{name}\u{201d} was open. Its autosave has \
         work the saved project doesn't{newer}."
    )
}

fn choice_button<'a>(
    label: &'a str,
    choice: RecoveryChoice,
    style: fn(button::Status) -> button::Style,
    primary: bool,
) -> button::Button<'a, Message> {
    let label = if primary {
        text(label)
            .size(13)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::BG_0)
    } else {
        text(label).size(13).color(theme::TEXT_1)
    };
    button(label)
        .on_press(Message::ProjectIo(ProjectIoMessage::RecoveryChoice(choice)))
        .padding([8, 18])
        .style(move |_theme, status| style(status))
}

pub(crate) fn view_recovery_prompt_overlay(prompt: &RecoveryPrompt) -> Element<'_, Message> {
    // Clicking the dimmed area is Cancel, like the quit confirmation.
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
    .on_press(Message::ProjectIo(ProjectIoMessage::RecoveryChoice(
        RecoveryChoice::Cancel,
    )));

    let title = text("Recover unsaved work?")
        .size(20)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1);
    let explanation = text(body_text(prompt)).size(13).color(theme::TEXT_2);
    let note = text(
        "A recovered project opens with unsaved changes \u{2014} save it to keep them. \
         The autosave stays on disk until then.",
    )
    .size(11)
    .color(theme::TEXT_3);

    let cancel_btn = choice_button(
        "Cancel",
        RecoveryChoice::Cancel,
        theme::ghost_button_style,
        false,
    );
    let other_btn = if prompt.untitled {
        choice_button(
            "Discard",
            RecoveryChoice::Discard,
            theme::destructive_button_style,
            false,
        )
    } else {
        choice_button(
            "Open last saved",
            RecoveryChoice::OpenLastSaved,
            theme::ghost_button_style,
            false,
        )
    };
    let recover_btn = choice_button(
        "Recover autosave",
        RecoveryChoice::RecoverAutosave,
        theme::primary_button_style,
        true,
    );

    let button_row = row![
        Space::new().width(Length::Fill),
        cancel_btn,
        other_btn,
        recover_btn
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let dialog_content = column![
        title,
        Space::new().height(10),
        explanation,
        Space::new().height(6),
        note,
        Space::new().height(20),
        button_row,
    ]
    .spacing(4)
    .padding(24)
    .width(480);

    let dialog = container(dialog_content).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: theme::RADIUS_XL.into(),
        },
        ..Default::default()
    });

    let centered = container(opaque(dialog))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    stack![backdrop, centered].into()
}
