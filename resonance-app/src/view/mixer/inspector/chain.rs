//! CHAIN group — plugin rows and the functional "+ Add to chain" /
//! "+ Add instrument" picker for the mixer inspector.

use iced::widget::{column, container, pick_list, row, text, Space};
use iced::{Element, Length};
use resonance_audio::types::{ScannedPlugin, TrackType};

use crate::message::{Message, PluginMessage};
use crate::state::TrackState;
use crate::theme;

pub(super) fn chain_group(
    r: &crate::Resonance,
    track: &TrackState,
    collapsed: bool,
) -> Element<'static, Message> {
    if collapsed {
        return super::widgets::group_header(
            "CHAIN",
            crate::state::MixerInspectorGroup::Chain,
            true,
        );
    }

    // 10px column spacing doubles as the title → first-row gap, so no
    // explicit spacer is needed after the group title.
    let mut col = column![super::widgets::group_header(
        "CHAIN",
        crate::state::MixerInspectorGroup::Chain,
        false,
    )]
    .spacing(10);

    // Instrument tracks render the instrument slot (plugin index 0) plus
    // any FX rows after it. Audio tracks render every plugin as an FX
    // row. Both end with the "+ FX" picker.
    //
    // External-instrument tracks are typed `Instrument` but their synth
    // is outboard hardware — there is no plugin slot to fill, and the
    // mixer runs every plugin on such a track as an insert effect over
    // the audio return. Offering an instrument slot here would build a
    // chain the engine renders differently from how it reads.
    let is_instrument = track.track_type == TrackType::Instrument
        && !r.external_instruments.contains_key(&track.id);
    if track.plugins.is_empty() {
        col = col.push(empty_chain_row());
    } else {
        for (i, plugin) in track.plugins.iter().enumerate() {
            let is_instrument_slot = is_instrument && i == 0;
            col = col.push(chain_row(&plugin.plugin_name, is_instrument_slot));
        }
    }

    // Functional add-plugin picker. Instrument tracks with an empty
    // chain get the instrument picker first; everyone else gets the FX
    // picker. Skipped when no plugins have been scanned yet. Options
    // come from `view_caches.{fx,instrument}_plugins` — Rc clones, not
    // a per-frame filter pass.
    let needs_instrument =
        is_instrument && track.plugins.is_empty() && track.sub_track.is_none();
    let candidates = if needs_instrument {
        r.view_caches.instrument_plugins.clone()
    } else {
        r.view_caches.fx_plugins.clone()
    };
    if !candidates.is_empty() {
        let track_id = track.id;
        let placeholder = if needs_instrument {
            "+ Add instrument"
        } else {
            "+ Add to chain"
        };
        let picker = pick_list(
            candidates,
            None::<ScannedPlugin>,
            move |plugin: ScannedPlugin| {
                Message::Plugin(PluginMessage::AddPluginToTrack(track_id, plugin))
            },
        )
        .placeholder(placeholder)
        .text_size(12)
        .padding([8, 10])
        .width(Length::Fill);
        col = col.push(picker);
    }

    col.into()
}

/// The dashed-looking "Empty chain" placeholder row. Shared with the bus
/// inspector so an empty track chain and an empty bus chain read the
/// same.
pub(super) fn empty_chain_row() -> Element<'static, Message> {
    container(text("Empty chain").size(11).color(theme::TEXT_3))
        .padding([8, 10])
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        })
        .into()
}

pub(super) fn chain_row(name: &str, is_instrument_slot: bool) -> Element<'static, Message> {
    let bullet_color = if is_instrument_slot {
        theme::ACCENT_SOFT
    } else {
        theme::ACCENT
    };
    let bullet = container(Space::new().width(0))
        .width(6)
        .height(6)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(bullet_color)),
            border: iced::Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        });
    let label_color = if is_instrument_slot {
        theme::ACCENT_SOFT
    } else {
        theme::TEXT_1
    };
    container(
        row![
            bullet,
            Space::new().width(8),
            text(name.to_string()).size(12).color(label_color),
            Space::new().width(Length::Fill),
            text("BYP")
                .size(9)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::TEXT_3),
        ]
        .align_y(iced::alignment::Vertical::Center),
    )
    .padding([8, 10])
    .width(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    })
    .into()
}
