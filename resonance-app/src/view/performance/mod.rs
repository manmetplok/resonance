//! Performance mode — full-screen, distraction-free live chord
//! teleprompter (epic #11, design doc #151).
//!
//! This module currently provides only the top-level dispatch stub: a
//! full-bleed surface routed to from [`crate::Resonance::view`] when
//! `view_mode == ViewMode::Performance`. The real status bar / center
//! stage / next-chords lane / footer are built in follow-up todos
//! (#307–#311); this scaffold exists so the mode can be entered and exited
//! while those surfaces land.

use crate::message::{Message, UiMessage};
use crate::theme;
use iced::widget::{button, column, container, text, Space};
use iced::{alignment, Element, Length};

impl crate::Resonance {
    /// Top-level Performance shell: a full-bleed container that owns the
    /// whole window (the normal transport chrome is hidden in this mode).
    /// Stub for todo #306 — the integrated Canvas teleprompter (status bar
    /// + center stage + next lane + footer) lands in todos #307+.
    pub(crate) fn view_performance_shell(&self) -> Element<'_, Message> {
        let exit_btn = button(
            text("Exit \u{238b}")
                .size(12)
                .font(theme::UI_FONT_MEDIUM)
                .color(theme::TEXT_2),
        )
        .on_press(Message::Ui(UiMessage::ExitPerformanceMode))
        .padding([6, 14])
        .style(|_theme, status| theme::ghost_button_style(status));

        let placeholder = column![
            text("PERFORMANCE")
                .size(13)
                .font(theme::UI_FONT_MEDIUM)
                .color(theme::ACCENT),
            Space::new().height(10),
            text("Live chord teleprompter")
                .size(20)
                .font(theme::SERIF_ITALIC_FONT)
                .color(theme::TEXT_1),
            Space::new().height(6),
            text("Press F or Esc to exit").size(13).color(theme::TEXT_3),
            Space::new().height(18),
            exit_btn,
        ]
        .align_x(alignment::Horizontal::Center);

        container(placeholder)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Center)
            .style(theme::base_bg)
            .into()
    }
}
