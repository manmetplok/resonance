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

const DIALOG_WIDTH: f32 = 560.0;
const MAX_RECENT_SHOWN: usize = 5;

pub(crate) fn view_startup_overlay(r: &Resonance) -> Element<'_, Message> {
    // Backdrop swallows pointer input; no on_press means clicks on
    // the dimmed area fall into the void, which is exactly what we
    // want for a non-dismissible modal.
    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.65,
                ))),
                ..Default::default()
            }),
    );

    // Brand mark + project title pair, mirroring the chrome layout so
    // the modal feels continuous with the app underneath.
    let brand_dot = text("\u{25cf}").size(13).color(theme::ACCENT);
    let brand = row![
        brand_dot,
        Space::new().width(8),
        text("Resonance")
            .size(14)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::TEXT_1),
    ]
    .align_y(alignment::Vertical::Center);

    let title = text("New session")
        .size(28)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1);
    let subtitle = text("Start a project to begin")
        .size(13)
        .color(theme::TEXT_3);

    let new_btn = primary_action(
        fa::FLOPPY_DISK,
        "New Project",
        Some(Message::Ui(UiMessage::StartNewProject)),
    );
    let open_btn = ghost_action(
        fa::FOLDER_OPEN,
        "Open Project...",
        Some(Message::ProjectIo(ProjectIoMessage::OpenProject)),
    );
    let template_btn = ghost_action(fa::MUSIC, "Start from Template... (coming soon)", None);

    let recent_label = text("RECENT PROJECTS")
        .size(10)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3);

    let recent_section: Element<'_, Message> = if r.io.recent_projects.is_empty() {
        text("No recent projects")
            .size(12)
            .color(theme::TEXT_3)
            .into()
    } else {
        let mut col = column![].spacing(4);
        for entry in r.io.recent_projects.iter().take(MAX_RECENT_SHOWN) {
            col = col.push(recent_row(entry));
        }
        scrollable(col).height(Length::Shrink).into()
    };

    let dialog_content = column![
        brand,
        Space::new().height(20),
        title,
        Space::new().height(2),
        subtitle,
        Space::new().height(24),
        new_btn,
        open_btn,
        template_btn,
        Space::new().height(24),
        recent_label,
        Space::new().height(8),
        recent_section,
    ]
    .spacing(6)
    .padding(32)
    .width(DIALOG_WIDTH);

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

fn primary_action<'a>(
    icon: char,
    label: &'a str,
    on_press: Option<Message>,
) -> iced::widget::Button<'a, Message> {
    let btn = button(
        row![
            theme::icon(icon).size(14).color(theme::BG_0),
            Space::new().width(10),
            text(label)
                .size(13)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::BG_0),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .padding([10, 16])
    .width(Length::Fill)
    .style(|_theme, status| theme::primary_button_style(status));
    match on_press {
        Some(msg) => btn.on_press(msg),
        None => btn,
    }
}

fn ghost_action<'a>(
    icon: char,
    label: &'a str,
    on_press: Option<Message>,
) -> iced::widget::Button<'a, Message> {
    let label_color = if on_press.is_some() {
        theme::TEXT_1
    } else {
        theme::TEXT_3
    };
    let btn = button(
        row![
            theme::icon(icon).size(13).color(label_color),
            Space::new().width(10),
            text(label).size(13).color(label_color),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .padding([10, 16])
    .width(Length::Fill)
    .style(|_theme, status| theme::ghost_button_style(status));
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
        text(entry.display_name.clone())
            .size(13)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::TEXT_1),
        text(parent_display).size(10).color(theme::TEXT_3),
    ]
    .spacing(2);
    button(
        row![
            theme::icon(fa::FOLDER_OPEN).size(12).color(theme::TEXT_3),
            Space::new().width(10),
            label,
        ]
        .align_y(alignment::Vertical::Center),
    )
    .padding([8, 12])
    .width(Length::Fill)
    .on_press(Message::ProjectIo(ProjectIoMessage::OpenRecent(
        entry.path.clone(),
    )))
    .style(|_theme, status| theme::ghost_button_style(status))
    .into()
}
