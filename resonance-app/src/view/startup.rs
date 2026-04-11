//! Startup / "no active project" modal. Shown over the DAW whenever
//! `io.has_active_project` is false — at boot and any later state
//! where no project is loaded. Offers New Project, Open Project,
//! Recent Projects, and a disabled Templates placeholder. No close
//! button: the user must pick something.
use iced::widget::{
    button, column, container, mouse_area, opaque, row, scrollable, stack, text, Space,
};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::recent::RecentEntry;
use crate::theme::{self, fa};
use crate::Resonance;

const DIALOG_WIDTH: u16 = 560;
const MAX_RECENT_SHOWN: usize = 5;

pub(crate) fn view_startup_overlay(r: &Resonance) -> Element<'_, Message> {
    // Backdrop swallows pointer input; no on_press means clicks on
    // the dimmed area fall into the void, which is exactly what we
    // want for a non-dismissible modal.
    let backdrop = mouse_area(
        container(Space::new(Length::Fill, Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.75,
                ))),
                ..Default::default()
            }),
    );

    let title = text("Resonance").size(36).color(theme::ACCENT);
    let subtitle = text("Start a project to begin").size(14).color(theme::TEXT_DIM);

    let new_btn = wide_button(
        fa::FLOPPY_DISK,
        "New Project",
        Some(Message::Ui(UiMessage::StartNewProject)),
    );
    let open_btn = wide_button(
        fa::FOLDER_OPEN,
        "Open Project...",
        Some(Message::ProjectIo(ProjectIoMessage::OpenProject)),
    );
    let template_btn = wide_button(fa::MUSIC, "Start from Template... (coming soon)", None);

    let recent_section: Element<'_, Message> = if r.io.recent_projects.is_empty() {
        text("No recent projects")
            .size(12)
            .color(theme::TEXT_DIM)
            .into()
    } else {
        let mut col = column![].spacing(4);
        for entry in r.io.recent_projects.iter().take(MAX_RECENT_SHOWN) {
            col = col.push(recent_row(entry));
        }
        scrollable(col).height(Length::Shrink).into()
    };

    let dialog_content = column![
        title,
        subtitle,
        Space::with_height(20),
        new_btn,
        open_btn,
        template_btn,
        Space::with_height(20),
        text("Recent Projects").size(11).color(theme::TEXT_DIM),
        Space::with_height(6),
        recent_section,
    ]
    .spacing(8)
    .padding(32)
    .width(DIALOG_WIDTH);

    let dialog = container(dialog_content).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::PANEL)),
        border: iced::Border {
            color: theme::SEPARATOR,
            width: 1.0,
            radius: 10.0.into(),
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

fn wide_button<'a>(
    icon: char,
    label: &'a str,
    on_press: Option<Message>,
) -> iced::widget::Button<'a, Message> {
    let btn = button(
        row![
            theme::icon(icon).size(14).color(theme::TEXT),
            Space::with_width(12),
            text(label).size(14).color(theme::TEXT),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .padding([10, 16])
    .width(Length::Fill)
    .style(|_theme, status| theme::transport_button_style(status));
    match on_press {
        Some(msg) => btn.on_press(msg),
        None => btn,
    }
}

fn recent_row(entry: &RecentEntry) -> Element<'_, Message> {
    let parent_display = entry
        .path
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let label = column![
        text(entry.display_name.clone()).size(13).color(theme::TEXT),
        text(parent_display).size(10).color(theme::TEXT_DIM),
    ]
    .spacing(2);
    button(
        row![
            theme::icon(fa::FOLDER_OPEN).size(12).color(theme::TEXT_DIM),
            Space::with_width(10),
            label,
        ]
        .align_y(alignment::Vertical::Center),
    )
    .padding([6, 12])
    .width(Length::Fill)
    .on_press(Message::ProjectIo(ProjectIoMessage::OpenRecent(
        entry.path.clone(),
    )))
    .style(|_theme, status| theme::transport_button_style(status))
    .into()
}
