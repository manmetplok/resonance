//! Track header column shown on the left of the Arrange view. Contains
//! a "+" add-track button plus one stacked header cell per visible track.
//! Sub-tracks are intentionally hidden here — they only make sense in the
//! mixer where they get per-bus routing.
use iced::widget::{button, column, container, row, text, Space};
use iced::{alignment, Element, Length};

use crate::message::Message;
use crate::state::TrackState;
use crate::theme;
use crate::view::controls::{
    delete_button, monitor_button, mono_button, mute_button, record_arm_button, solo_button,
};
use crate::Resonance;

pub(crate) fn view_track_headers(r: &Resonance) -> Element<'_, Message> {
    let mut headers = column![].spacing(0);

    // Ruler header with "+" button to add a track
    let add_btn = button(text("+").size(16).color(theme::TEXT))
        .on_press(Message::OpenAddTrackMenu)
        .style(|_theme, status| theme::small_button_style(status))
        .padding([0, 6]);
    let add_row = row![Space::with_width(6), add_btn]
        .align_y(alignment::Vertical::Center)
        .height(theme::RULER_HEIGHT);
    headers = headers.push(
        container(add_row)
            .width(Length::Fill)
            .height(theme::RULER_HEIGHT)
            .style(theme::panel_dark_bg),
    );

    let sorted_tracks: Vec<&TrackState> = r
        .sorted_tracks()
        .into_iter()
        .filter(|t| t.sub_track.is_none())
        .collect();

    // Calculate which tracks are visible given scroll_offset_y
    let visible_start = r.viewport.scroll_offset_y / theme::TRACK_HEIGHT;
    let first_visible = visible_start.floor() as usize;
    // Add top padding for the scrolled-away portion
    let top_pad = first_visible as f32 * theme::TRACK_HEIGHT - r.viewport.scroll_offset_y;
    if first_visible > 0 {
        headers = headers.push(Space::new(
            Length::Fill,
            (first_visible as f32 * theme::TRACK_HEIGHT - r.viewport.scroll_offset_y).max(0.0),
        ));
    } else if r.viewport.scroll_offset_y > 0.0 {
        headers = headers.push(Space::new(Length::Fill, top_pad.max(0.0)));
    }

    for (i, track) in sorted_tracks.iter().enumerate() {
        if i < first_visible {
            continue;
        }
        headers = headers.push(view_track_header(track));
    }

    container(headers)
        .width(theme::TRACK_HEADER_WIDTH)
        .height(Length::Fill)
        .clip(true)
        .style(theme::panel_outlined)
        .into()
}

fn view_track_header(track: &TrackState) -> Element<'_, Message> {
    let is_sub = track.sub_track.is_some();
    // Track name on its own line, clipped at the header width so long
    // names don't push the icons offscreen. `Wrapping::None` prevents
    // iced from line-wrapping and the enclosing container's clip flag
    // trims any glyph that overflows the available width. Sub-tracks
    // render dimmer since they're driven by their parent plugin.
    let name_color = if is_sub {
        theme::TEXT_DIM
    } else {
        theme::TEXT
    };
    let name = text(track.name.clone())
        .size(13)
        .color(name_color)
        .wrapping(iced::widget::text::Wrapping::None);
    let name_row = container(name).width(Length::Fill).clip(true);

    // Sub-tracks expose a trimmed toolbar: just mute/solo + a per-port
    // label. They cannot be armed, monitored, deleted, or swapped to
    // mono — those all belong to their parent.
    let icon_row: iced::widget::Row<'_, Message> = if is_sub {
        row![
            mute_button(track.muted, Message::ToggleMute(track.id), 12),
            solo_button(track.soloed, Message::ToggleSolo(track.id), 12),
            Space::with_width(Length::Fill),
        ]
        .spacing(4)
        .align_y(alignment::Vertical::Center)
    } else {
        row![
            mono_button(track.mono, track.id, 12),
            monitor_button(track.monitor_enabled, track.id, 12),
            record_arm_button(track.record_armed, track.id, 12),
            mute_button(track.muted, Message::ToggleMute(track.id), 12),
            solo_button(track.soloed, Message::ToggleSolo(track.id), 12),
            delete_button(Message::RemoveTrack(track.id), 12),
            Space::with_width(Length::Fill),
        ]
        .spacing(4)
        .align_y(alignment::Vertical::Center)
    };

    let header_col = column![name_row, icon_row].spacing(4);

    // Sub-tracks get an indent on the left so their visual hierarchy
    // under the parent track is obvious at a glance.
    let left_pad: f32 = if is_sub { 20.0 } else { 8.0 };
    let content = container(header_col)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(iced::Padding {
            top: 6.0,
            right: 8.0,
            bottom: 6.0,
            left: left_pad,
        })
        .clip(true);

    let bg = if track.record_armed {
        theme::PANEL_ARMED
    } else if is_sub {
        theme::PANEL
    } else {
        theme::PANEL_DARK
    };
    let border_color = if track.record_armed {
        theme::RECORD_RED
    } else {
        theme::SEPARATOR
    };

    container(content)
        .width(Length::Fill)
        .height(theme::TRACK_HEIGHT)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(bg)),
            border: iced::Border {
                color: border_color,
                width: 0.5,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}
