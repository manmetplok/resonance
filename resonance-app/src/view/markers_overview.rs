//! Arrangement-markers overview popover (todo #370 / doc #161).
//!
//! A togglable list anchored just under the transport bar, opened by the
//! transport's flag button. Each row shows the marker's colour swatch, its
//! name, and its bar position; clicking a row seeks the playhead to that
//! marker (`MarkerMessage::JumpTo`). The list stays open across jumps so a
//! user can audition several sections in a row; a backdrop click dismisses
//! it. Nav between markers also has transport buttons + `.`/`,` shortcuts
//! (see `view::transport` and `update.rs`).
//!
//! The overview is a pure function of `Resonance::markers` + the tempo map
//! (for the bar-position labels), rendered only while the popover is open,
//! so it costs nothing on the hot paint path when closed.

use iced::widget::{
    button, column, container, mouse_area, opaque, row, scrollable, stack, text, Space,
};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::state::ArrangementMarker;
use crate::theme;
use crate::Resonance;

/// Width of the overview popover panel.
const PANEL_WIDTH: f32 = 260.0;

/// Build the overview overlay: a dimmed backdrop that closes the popover on
/// click, with the marker list positioned under the transport's flag button.
pub(crate) fn view_markers_overview_overlay(r: &Resonance) -> Element<'_, Message> {
    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.3,
                ))),
                ..Default::default()
            }),
    )
    .on_press(Message::Ui(UiMessage::CloseMarkersOverview));

    let header = text("MARKERS")
        .size(10)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3);

    let mut list = column![header, Space::new().height(6)].spacing(2);

    if r.markers.is_empty() {
        list = list.push(
            text("No markers yet")
                .size(12)
                .color(theme::TEXT_3),
        );
    } else {
        for marker in r.markers.as_slice() {
            list = list.push(marker_row(r, marker));
        }
    }

    let panel = container(opaque(scrollable(list.padding(10)).height(Length::Shrink)))
        .width(PANEL_WIDTH)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_LG.into(),
            },
            ..Default::default()
        });

    // Anchor the panel just below the transport bar, near the left cluster
    // where the flag button lives.
    let top_pad = super::transport::CHROME_HEIGHT + super::transport::TRANSPORT_HEIGHT + 2.0;
    let positioned = container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Left)
        .align_y(alignment::Vertical::Top)
        .padding(iced::Padding {
            top: top_pad,
            right: 0.0,
            bottom: 0.0,
            left: 30.0,
        });

    stack![backdrop, positioned].into()
}

/// One clickable overview entry: colour swatch + name + bar position. The
/// whole row is a button that seeks the playhead to the marker start.
fn marker_row<'a>(r: &Resonance, marker: &'a ArrangementMarker) -> Element<'a, Message> {
    let swatch = container(Space::new().width(10).height(10)).style(move |_theme| {
        container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb8(
                marker.color[0],
                marker.color[1],
                marker.color[2],
            ))),
            border: iced::Border {
                radius: 2.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    });

    // A "◇" glyph distinguishes ranged region markers from point flags.
    let kind = if marker.is_region() { "\u{25c7} " } else { "" };

    let pos_label = text(format_bar_position(r, marker.start_sample))
        .size(11)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3);

    let row_content = row![
        container(swatch)
            .align_y(alignment::Vertical::Center)
            .height(Length::Fill),
        Space::new().width(8),
        text(format!("{kind}{}", marker.name))
            .size(12)
            .color(theme::TEXT_1),
        Space::new().width(Length::Fill),
        pos_label,
    ]
    .align_y(alignment::Vertical::Center);

    button(row_content)
        .on_press(Message::Marker(MarkerMessage::JumpTo(marker.id)))
        .width(Length::Fill)
        .padding([5, 8])
        .style(|_theme, status| theme::transport_button_style(status))
        .into()
}

/// Format a sample position as a `bar.beat` label using the live tempo map,
/// matching the transport position readout's 1-based bar/beat convention.
fn format_bar_position(r: &Resonance, sample: u64) -> String {
    let (bar_0, frac) = r.tempo_map.sample_to_bar(sample, r.sample_rate);
    let beats = frac * r.transport.time_sig_num as f64;
    format!("{}.{}", bar_0 + 1, beats.floor() as u32 + 1)
}
