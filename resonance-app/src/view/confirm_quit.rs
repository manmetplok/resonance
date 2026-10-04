//! Confirmation dialogs for leaving a project with unsaved changes: closing
//! the window, and replacing the project through Open / New (code review
//! UX-01). Both follow the same backdrop + centered-dialog pattern as the
//! confirm-delete-track overlay, and share [`unsaved_changes_dialog`].
use iced::widget::{button, column, container, mouse_area, opaque, row, stack, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::state::ProjectSwitch;
use crate::theme;
use crate::Resonance;

pub(crate) fn view_confirm_quit_overlay<'a>(_r: &'a Resonance) -> Element<'a, Message> {
    unsaved_changes_dialog(
        "You have unsaved changes. What would you like to do?".to_string(),
        Message::ProjectIo(ProjectIoMessage::CancelQuit),
        ("Discard & Quit", Message::ProjectIo(ProjectIoMessage::ConfirmDiscardAndQuit)),
        ("Save & Quit", Message::ProjectIo(ProjectIoMessage::ConfirmSaveAndQuit)),
    )
}

/// The Save / Don't save / Cancel dialog a GUI Open or New raises over
/// unsaved changes (code review UX-01). "Save" on an untitled project goes
/// through Save As, then carries on.
pub(crate) fn view_confirm_switch_overlay<'a>(
    r: &'a Resonance,
    switch: &ProjectSwitch,
) -> Element<'a, Message> {
    let current = r
        .io
        .project_path
        .as_ref()
        .and_then(|p| p.file_stem())
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled");
    let action = match switch {
        ProjectSwitch::Open(path) => {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("the project");
            format!("opening \u{201c}{name}\u{201d}")
        }
        ProjectSwitch::NewEmpty => "starting a new project".to_string(),
    };
    let save_label = if r.io.project_path.is_some() {
        "Save"
    } else {
        "Save As\u{2026}"
    };
    let choice = |c| Message::ProjectIo(ProjectIoMessage::SwitchChoice(c));
    unsaved_changes_dialog(
        format!(
            "Save the changes to \u{201c}{current}\u{201d} before {action}? \
             Unsaved changes are lost otherwise."
        ),
        choice(SwitchChoice::Cancel),
        ("Don't Save", choice(SwitchChoice::Discard)),
        (save_label, choice(SwitchChoice::Save)),
    )
}

/// The shared unsaved-changes dialog: a title, `explanation`, and Cancel /
/// destructive / primary buttons. A backdrop click sends `cancel` too.
fn unsaved_changes_dialog<'a>(
    explanation: String,
    cancel: Message,
    (discard_label, discard): (&'static str, Message),
    (save_label, save): (&'static str, Message),
) -> Element<'a, Message> {
    // Backdrop swallows pointer input so the DAW behind is inert while
    // the dialog is up. Clicking the dimmed area cancels.
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
    .on_press(cancel.clone());

    let title = text("Unsaved changes")
        .size(20)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1);
    let explanation = text(explanation).size(13).color(theme::TEXT_2);

    let cancel_btn = button(text("Cancel").size(13).color(theme::TEXT_1))
        .on_press(cancel)
        .padding([8, 18])
        .style(|_theme, status| theme::ghost_button_style(status));

    let discard_btn = button(text(discard_label).size(13).color(theme::TEXT_1))
        .on_press(discard)
        .padding([8, 18])
        .style(|_theme, status| theme::destructive_button_style(status));

    let save_btn = button(
        text(save_label)
            .size(13)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::BG_0),
    )
    .on_press(save)
    .padding([8, 18])
    .style(|_theme, status| theme::primary_button_style(status));

    let button_row = row![
        Space::new().width(Length::Fill),
        cancel_btn,
        discard_btn,
        save_btn
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let dialog_content = column![
        title,
        Space::new().height(10),
        explanation,
        Space::new().height(20),
        button_row,
    ]
    .spacing(4)
    .padding(24)
    .width(440);

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
