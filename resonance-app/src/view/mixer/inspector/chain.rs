//! CHAIN group — plugin rows and the functional "+ Add to chain" /
//! "+ Add instrument" picker for the mixer inspector.

use iced::widget::{button, column, container, pick_list, row, text, Space};
use iced::{Element, Length};
use resonance_audio::types::{ScannedPlugin, TrackType};

use crate::message::{Message, PluginMessage, PresetAddOwner, PresetUiMessage};
use crate::state::TrackState;
use crate::theme;
use crate::view::mixer::picks::PluginOwner;
use crate::view::mixer::reorder::{self, ChainMoves};

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
        && !r.devices.external_instruments.contains_key(&track.id);
    if track.plugins.is_empty() {
        col = col.push(empty_chain_row());
    } else {
        let chain_len = track.plugins.len();
        for (i, plugin) in track.plugins.iter().enumerate() {
            let is_instrument_slot = is_instrument && i == 0;
            let moves = reorder::chain_moves(
                r,
                PluginOwner::Track(track.id),
                plugin.instance_id,
                i,
                chain_len,
            );
            col = col.push(chain_row(
                &plugin.plugin_name,
                is_instrument_slot,
                &moves,
                plugin.instance_id,
                plugin.bypassed,
            ));
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
        r.ui.view_caches.instrument_plugins.clone()
    } else {
        r.ui.view_caches.fx_plugins.clone()
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
        // "▸ with preset…": the user's favourite presets of the same kind
        // of plugin (§6.6), precomputed on a scan or a star.
        let picks = if needs_instrument {
            r.presets.instrument_favorite_picks.clone()
        } else {
            r.presets.fx_favorite_picks.clone()
        };
        if !picks.is_empty() {
            let with_preset = pick_list(
                picks,
                None::<crate::state::presets::PresetAddPick>,
                move |pick| {
                    Message::Plugin(PluginMessage::PresetUi(PresetUiMessage::AddWithPreset {
                        owner: PresetAddOwner::Track(track_id),
                        pick,
                    }))
                },
            )
            .placeholder("\u{25b8} with preset\u{2026}")
            .text_size(12)
            .padding([8, 10])
            .width(Length::Fill);
            col = col.push(with_preset);
        }
    }

    col.into()
}

/// The "Empty chain" placeholder row. Shared with the bus and master
/// inspectors so every empty chain reads the same.
pub(super) fn empty_chain_row() -> Element<'static, Message> {
    super::widgets::placeholder_row("Empty chain")
}

/// The BYP control: a real toggle since ba todo #1305.
///
/// It was a `text("BYP")` label for a long time — the chain row LOOKED
/// like it offered per-plugin bypass and did nothing, which is the
/// audit's finding X3. Sends a SET rather than a toggle so the wire and
/// the button raise the identical message, and lights up when the slot
/// is bypassed so the state is readable without hovering.
fn bypass_button(
    instance_id: resonance_audio::types::PluginInstanceId,
    bypassed: bool,
) -> Element<'static, Message> {
    let label = text("BYP")
        .size(9)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(if bypassed {
            theme::BG_1
        } else {
            theme::TEXT_3
        });
    button(label)
        .padding([2, 5])
        .on_press(Message::Plugin(
            crate::message::PluginMessage::SetPluginBypass {
                instance_id,
                bypassed: !bypassed,
            },
        ))
        .style(move |_theme, status| {
            let bg = match (bypassed, status) {
                (true, _) => theme::WARM,
                (false, iced::widget::button::Status::Hovered) => theme::BG_3,
                (false, _) => iced::Color::TRANSPARENT,
            };
            iced::widget::button::Style {
                background: Some(iced::Background::Color(bg)),
                text_color: if bypassed { theme::BG_1 } else { theme::TEXT_3 },
                border: iced::Border {
                    color: if bypassed { theme::WARM } else { theme::LINE_2 },
                    width: 1.0,
                    radius: theme::RADIUS_SM.into(),
                },
                ..Default::default()
            }
        })
        .into()
}

/// One plugin row of a CHAIN group: bullet · name · ▲▼ · BYP.
///
/// `moves` carries the two reorder messages for this slot (ba todo
/// #1302); either side is `None` when the move is unavailable, and the
/// caret then renders greyed and unclickable rather than raising a move
/// the chain rule would refuse.
pub(super) fn chain_row(
    name: &str,
    is_instrument_slot: bool,
    moves: &ChainMoves,
    instance_id: resonance_audio::types::PluginInstanceId,
    bypassed: bool,
) -> Element<'static, Message> {
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
            reorder::move_buttons(moves, 10.0, 3),
            Space::new().width(8),
            bypass_button(instance_id, bypassed),
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
