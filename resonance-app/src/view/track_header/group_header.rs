//! Track-group (folder-track) header row for the Arrange view track-header
//! column (epic #36, doc #200). A 60 px presentational band, distinct from
//! the 96 px per-track cell: it folds related lanes under one foldable
//! header, brackets them with a colour identity, and exposes group-level
//! macro controls (mute / solo / level trim) that cascade to members.
//!
//! Anatomy left→right (DESIGN.md key decision #1):
//!   caret · colour swatch · bold name · `N trk` count badge ·
//!   group level trim (mini-slider + dB) · macro `M` · macro `S`
//! over a faint group-colour wash so the row reads as a band.
//!
//! This is the VIEW only (todo #680): it emits [`GroupMessage`]s but
//! mutates nothing. The reducers behind the controls land in #686–#689,
//! and wiring the row into the live timeline column belongs to #681/#682/
//! #686 — this component is verified standalone via a snapshot test.
use iced::widget::{container, mouse_area, row, slider, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::theme;

/// Render a single group-header row (60 px) for `group`, showing
/// `member_count` in the count badge. Returns a `'static` element, so all
/// data is cloned/copied out of `group` (mirrors `view_track_header`).
pub(crate) fn view_group_header(
    group: &resonance_common::track_group::TrackGroup,
    member_count: usize,
) -> Element<'static, Message> {
    let group_id = group.id;
    let (base, wash, line) = theme::group_identity_colors(group.identity_color);

    // ---- Caret (folds the group) ----
    // The caret is the expand/collapse affordance: ▾ when the group is
    // expanded (members shown), ▸ when collapsed. `collapse_caret` takes
    // `expanded`, so pass `!is_collapsed`.
    let caret = mouse_area(crate::view::controls::collapse_caret(!group.is_collapsed))
        .on_press(Message::Group(GroupMessage::ToggleCollapse(group_id)));

    // ---- Identity colour swatch (14 px rounded square, identity base) ----
    let swatch = container(Space::new())
        .width(14)
        .height(14)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(base)),
            border: iced::Border {
                color: line,
                width: 1.0,
                radius: theme::RADIUS_XS.into(),
            },
            ..Default::default()
        });

    // ---- Bold group name ----
    let name = text(group.name.clone())
        .size(13)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_1)
        .wrapping(iced::widget::text::Wrapping::None);

    // ---- `N trk` count badge (pill — mirrors shelf.rs's count badge) ----
    let count_badge = container(
        text(format!("{member_count} trk"))
            .size(10)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_3),
    )
    .padding(iced::Padding {
        top: 1.0,
        right: 7.0,
        bottom: 1.0,
        left: 7.0,
    })
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    });

    // ---- Group level trim: compact slider + dB readout ----
    // The trim is a multiplicative macro gain (`1.0` is unity). The slider
    // spans 0.0..=2.0 around unity; the readout shows dB (−∞ at silence).
    let level = group.macro_level;
    let level_slider = slider(0.0..=2.0f32, level, move |v| {
        Message::Group(GroupMessage::SetMacroLevel(group_id, v))
    })
    .step(0.01f32)
    .width(70);
    let db_text = if level <= 0.0001 {
        "−∞ dB".to_string()
    } else {
        format!("{:.1} dB", 20.0 * level.log10())
    };
    let db_label = text(db_text)
        .size(9)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_2);
    let level_trim = row![level_slider, db_label]
        .spacing(5)
        .align_y(alignment::Vertical::Center);

    // ---- Macro M / S buttons (reuse BAD/WARM-styled controls) ----
    let m_btn = crate::view::controls::mute_button(
        group.macro_mute,
        Message::Group(GroupMessage::ToggleMacroMute(group_id)),
        12,
    );
    let s_btn = crate::view::controls::solo_button(
        group.macro_solo,
        Message::Group(GroupMessage::ToggleMacroSolo(group_id)),
        12,
    );

    // ---- Assemble the row ----
    let inner = row![
        caret,
        Space::new().width(7),
        swatch,
        Space::new().width(7),
        name,
        Space::new().width(7),
        count_badge,
        Space::new().width(Length::Fill),
        level_trim,
        Space::new().width(6),
        m_btn,
        s_btn,
        Space::new().width(8),
    ]
    .spacing(0)
    .align_y(alignment::Vertical::Center)
    .padding(iced::Padding {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 16.0,
    });

    // Faint group-colour wash background so the row reads as a band.
    let body = container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(wash)),
            ..Default::default()
        });

    // The cell is `GROUP_HEADER_HEIGHT - 1` so that `cell + hairline`
    // together sum to exactly `GROUP_HEADER_HEIGHT` — matching the
    // canvas's per-row pitch (same trick as `view_track_header`).
    let cell = container(body)
        .width(Length::Fill)
        .height(theme::GROUP_HEADER_HEIGHT - 1.0);

    // 1px hairline below so the row separates without a heavy border.
    let hairline = container(Space::new().width(Length::Fill))
        .height(1)
        .style(theme::separator_bg);

    iced::widget::column![cell, hairline].spacing(0).into()
}
