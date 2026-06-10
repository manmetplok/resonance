//! Per-track channel strip (and the collapsed-parent meters and the
//! output-destination picker that hangs off each strip).

use iced::widget::{button, column, container, mouse_area, pick_list, row, scrollable, text, Space};
use iced::{alignment, Element, Font, Length};
use resonance_audio::types::*;

use crate::message::*;
use crate::state::*;
use crate::theme::{self, fa};
use crate::util::format_pan;
use crate::view::controls::{
    bounce_button, fader_section, fx_bypass_button, meter_v, monitor_button, mono_button,
    mute_button, record_arm_button, solo_button,
};
use crate::view::knob::pan_knob;

use super::picks::PluginOwner;

impl crate::Resonance {
    pub(super) fn view_channel_strip(
        &self,
        track: &TrackState,
        available_plugins: &[ScannedPlugin],
    ) -> Element<'_, Message> {
        // Sub-tracks never reach this function — view_mixer skips them
        // in its outer loop and renders them via view_sub_channel_strip
        // (the slimmer variant) inside their parent's cluster instead.
        debug_assert!(
            track.sub_track.is_none(),
            "view_channel_strip called with a sub-track; use view_sub_channel_strip"
        );

        // Parent instrument tracks that have at least one sub-track show
        // a small collapse/expand button next to the name. Clicking it
        // toggles `expanded_sub_track_parents`, which view_mixer reads
        // before rendering each sub-track strip.
        let has_sub_tracks = self
            .registry
            .tracks
            .iter()
            .any(|t| matches!(t.sub_track, Some(link) if link.parent_track_id == track.id));
        let is_collapsed =
            has_sub_tracks && !self.mixer.expanded_sub_track_parents.contains(&track.id);

        // Track names that overflow the 132 px strip get an ellipsis so
        // they don't push onto a second line. Wrapping::None alone isn't
        // enough — Iced still wraps when the parent has finite width.
        // Truncate first, then clip in a width-Fill container.
        let display_name = crate::util::short(&track.name, 14);
        let name_text = container(
            text(display_name)
                .size(12)
                .font(theme::UI_FONT_MEDIUM)
                .color(theme::TEXT)
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .width(Length::Fill)
        .clip(true);

        // Strip head: 22×22 lavender glyph + name on a bottom-bordered
        // row. Matches the redesign's `.stripHead` block.
        let glyph_char = match track.track_type {
            TrackType::Audio => fa::MICROPHONE,
            TrackType::Instrument => track.instrument_icon.glyph(),
            TrackType::Vocal => fa::MICROPHONE,
        };
        let head_glyph: Element<'_, Message> = container(
            theme::icon(glyph_char)
                .size(11)
                .color(theme::ACCENT_SOFT),
        )
        .width(22)
        .height(22)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_3)),
            border: iced::Border {
                radius: theme::RADIUS_SM.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into();

        let mut head_row = row![]
            .spacing(8)
            .align_y(alignment::Vertical::Center)
            .height(28);
        if has_sub_tracks {
            let glyph_caret = if is_collapsed {
                fa::CARET_RIGHT
            } else {
                fa::CARET_DOWN
            };
            let track_id = track.id;
            let toggle = button(theme::icon(glyph_caret).size(10).color(theme::TEXT_3))
                .on_press(Message::Track(TrackMessage::ToggleSubTracksVisible(
                    track_id,
                )))
                .padding([2, 4])
                .style(|_theme, status| theme::small_button_style(status));
            head_row = head_row.push(toggle);
        }
        head_row = head_row.push(head_glyph);
        head_row = head_row.push(name_text);
        let track_name: Element<'_, Message> = container(head_row)
            .width(Length::Fill)
            .padding([6, 10])
            .style(strip_head_bg)
            .into();

        // Two-row control block: the design's M / S / ● / 🎧 quartet up
        // top, then a smaller utility row with mono / FX bypass /
        // (optional) bounce. Splitting prevents the 6+ buttons from
        // overflowing the 132px strip width and keeps the dominant row
        // visually consistent with the Arrange track header.
        let bounce_enabled = crate::update::track::classify_bounce(
            track,
            self.midi_clips.iter().map(|c| c.track_id),
        )
        .is_ok();

        let primary_row = row![
            mute_button(
                track.muted,
                Message::Track(TrackMessage::ToggleMute(track.id)),
                12
            ),
            solo_button(
                track.soloed,
                Message::Track(TrackMessage::ToggleSolo(track.id)),
                12
            ),
            record_arm_button(track.record_armed, track.id, 12),
            monitor_button(track.monitor_enabled, track.id, 12),
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center);

        let mut utility_row = row![
            mono_button(track.mono, track.id, 11),
            fx_bypass_button(
                track.fx_bypassed,
                Message::Track(TrackMessage::ToggleTrackFxBypass(track.id)),
                11,
            ),
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center);
        if track.track_type == TrackType::Instrument {
            utility_row = utility_row.push(bounce_button(track.id, bounce_enabled, 11));
        }

        // Two stacked rows of icon buttons. Iced's column macro collapses
        // when height is unconstrained inside a strip column whose total
        // height is Length::Fill, so we pin a fixed height that fits the
        // 20+19+spacing block.
        let button_row: Element<'_, Message> = container(
            column![
                container(primary_row).center_x(Length::Fill),
                container(utility_row).center_x(Length::Fill),
            ]
            .spacing(2),
        )
        .width(Length::Fill)
        .height(Length::Fixed(48.0))
        .padding([2, 0])
        .into();

        // Output destination + per-track routing now live in the
        // Inspector — no per-strip pickers are constructed here.

        // Instrument slot and FX chain are now two independent sections:
        // the instrument pill stays fixed at the top of the plugin area
        // and the FX list scrolls below it, so a track with many FX never
        // pushes the fader off the strip.
        let is_instrument_track = track.track_type == TrackType::Instrument;

        let instrument_section: Option<Element<'_, Message>> =
            if is_instrument_track {
                if let Some(plugin) = track.plugins.first() {
                    Some(self.view_plugin_slot_row(
                        PluginOwner::Track(track.id),
                        plugin,
                        true,
                    ))
                } else if !self.view_caches.instrument_plugins.is_empty() {
                    let track_id = track.id;
                    let inst_picker = pick_list(
                        self.view_caches.instrument_plugins.clone(),
                        None::<ScannedPlugin>,
                        move |plugin: ScannedPlugin| {
                            Message::Plugin(PluginMessage::AddPluginToTrack(track_id, plugin))
                        },
                    )
                    .placeholder("+ Instrument")
                    .text_size(10)
                    .width(Length::Fill);
                    Some(inst_picker.into())
                } else if available_plugins.is_empty() {
                    None
                } else {
                    Some(text("No instruments").size(9).color(theme::TEXT_DIM).into())
                }
            } else {
                None
            };

        // FX list: every plugin except the instrument slot (index 0) on
        // instrument tracks. Audio tracks render every plugin here.
        // Built into its own column so we can wrap it in a vertical
        // scrollable below.
        let mut fx_column = column![].spacing(2).width(Length::Fill);
        let fx_iter: Box<dyn Iterator<Item = &PluginSlotState>> = if is_instrument_track {
            Box::new(track.plugins.iter().skip(1))
        } else {
            Box::new(track.plugins.iter())
        };
        for plugin in fx_iter {
            fx_column = fx_column.push(self.view_plugin_slot_row(
                PluginOwner::Track(track.id),
                plugin,
                false,
            ));
        }

        // +FX picker, input/output pickers, and MIDI routing all live in
        // the Inspector now. The strip stays focused on M/S/●/🎧, the
        // instrument slot pill, pan, and the fader.
        let _ = available_plugins;

        // Pan knob — vertical drag to change, double-click to reset.
        let id = track.id;
        let pan_ctrl = pan_knob(track.pan, move |v| {
            Message::Track(TrackMessage::SetTrackPan(id, v))
        });
        let pan_label = format_pan(track.pan);
        let pan_row = row![
            text("Pan").size(9).color(theme::TEXT_DIM),
            Space::new().width(Length::Fill),
            pan_ctrl,
            Space::new().width(Length::Fill),
            text(pan_label)
                .size(9)
                .font(Font::MONOSPACE)
                .color(theme::TEXT_DIM),
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center);

        // Just the pan row — the FX picker moved to the Inspector.
        let fx_pan_block = iced::widget::Column::new()
            .width(Length::Fill)
            .push(pan_row);

        let track_id_for_fader = track.id;
        let fader_block = fader_section(track.level_l, track.level_r, track.volume, move |v| {
            Message::Track(TrackMessage::SetTrackVolume(track_id_for_fader, v))
        });

        // Input device + port + MIDI routing all live in the Inspector
        // now. The strip stays compact: head, button rows, instrument
        // pill, pan, fader. Sub-tracks remain the exception — they
        // still have no routing pickers anywhere.

        let is_selected = self.interaction.selected_track == Some(track.id);
        let bg = if track.record_armed {
            theme::PANEL_ARMED
        } else {
            theme::BG_2
        };
        let border_color = if track.record_armed {
            theme::BAD
        } else if is_selected {
            theme::ACCENT_LINE
        } else {
            theme::LINE_2
        };
        let border_w = if is_selected || track.record_armed {
            1.0
        } else {
            0.5
        };

        // FX list lives inside a vertical scrollable that absorbs all
        // remaining vertical slack between the buttons/instrument and
        // the fader. Overflowing FX rows scroll instead of pushing the
        // fader off the strip — the strip itself stays a fixed height.
        let fx_scroll = iced::widget::Scrollable::with_direction(
            fx_column,
            scrollable::Direction::Vertical(scrollable::Scrollbar::default().width(4).scroller_width(4)),
        )
        .width(Length::Fill)
        .height(Length::Fill);

        let mut plugin_column = column![].spacing(2).width(Length::Fill).height(Length::Fill);
        if let Some(inst) = instrument_section {
            plugin_column = plugin_column.push(inst);
            plugin_column = plugin_column
                .push(container(Space::new().width(Length::Fill).height(1)).style(theme::separator_bg));
        }
        plugin_column = plugin_column.push(fx_scroll);

        let plugin_fill: Element<'_, Message> = container(plugin_column)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();

        let strip_style = move |_theme: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(bg)),
            border: iced::Border {
                color: border_color,
                width: border_w,
                radius: theme::RADIUS_XL.into(),
            },
            ..Default::default()
        };

        let track_id_for_select = track.id;
        let strip_height = Length::Fixed(theme::MIXER_STRIP_HEIGHT as f32);
        if is_collapsed {
            // Two-column layout: normal controls on the left, compact
            // subtrack meters on the right. The strip widens by 30px per
            // sub-track output so a kit with N pads has every meter
            // visible at a glance instead of crammed into a fixed area.
            let subtrack_count = self
                .registry
                .tracks
                .iter()
                .filter(|t| matches!(t.sub_track, Some(link) if link.parent_track_id == track.id))
                .count() as u32;
            let right_col_w: f32 = ((subtrack_count.max(1) * 30) + 8) as f32;

            let left_col = column![
                track_name,
                button_row,
                plugin_fill,
                fx_pan_block,
                fader_block,
            ]
            .spacing(4)
            .width(theme::MIXER_STRIP_WIDTH - 12.0)
            .height(Length::Fill);

            let v_sep = container(Space::new().width(1).height(Length::Fill)).style(theme::separator_bg);

            let right_col = container(self.view_collapsed_subtrack_meters(track.id))
                .width(Length::Fixed(right_col_w))
                .height(Length::Fill)
                .padding([0, 4]);

            let strip_content = row![left_col, v_sep, right_col]
                .height(Length::Fill)
                .padding(6)
                .width(theme::MIXER_STRIP_WIDTH + right_col_w + 12.0);

            mouse_area(
                container(strip_content)
                    .height(strip_height)
                    .style(strip_style),
            )
            .on_press(Message::Ui(UiMessage::SelectTrack(Some(track_id_for_select))))
            .into()
        } else {
            let strip_content = column![
                track_name,
                button_row,
                plugin_fill,
                fx_pan_block,
                fader_block,
            ]
            .spacing(4)
            .padding(6)
            .width(theme::MIXER_STRIP_WIDTH)
            .height(Length::Fill);

            mouse_area(
                container(strip_content)
                    .height(strip_height)
                    .style(strip_style),
            )
            .on_press(Message::Ui(UiMessage::SelectTrack(Some(track_id_for_select))))
            .into()
        }
    }

    /// Dedicated narrow strip for an expanded sub-track. Mixer-mod.rs
    /// emits one of these immediately after the parent strip for each
    /// sub-track of an expanded parent, so the cluster (parent + its
    /// sub-strips) reads as a coherent group.
    ///
    /// Visual contract:
    /// - `MIXER_SUB_STRIP_WIDTH`-wide (narrower than the parent strip)
    ///   and `MIXER_STRIP_HEIGHT` tall (same as parent — fader bottoms
    ///   line up).
    /// - Background = `MIXER_SUB_STRIP_BG` (one step darker than the
    ///   parent strip's `BG_2`) so the recessed shade signals "child".
    /// - 2 px lavender left-edge rail (`MIXER_SUB_STRIP_RAIL`,
    ///   saturating to `_SELECTED` when the sub-track is the
    ///   selected track) — the at-a-glance parent → child cue.
    /// - Slimmer control set: M / S / Mute-mono, Pan, fader. No
    ///   record-arm or monitor (sub-tracks are fed from the parent
    ///   plugin's fan-out, never from a hardware input), no FX list,
    ///   no instrument pill, no bounce.
    pub(super) fn view_sub_channel_strip(
        &self,
        track: &TrackState,
        _available_plugins: &[ScannedPlugin],
    ) -> Element<'_, Message> {
        debug_assert!(
            track.sub_track.is_some(),
            "view_sub_channel_strip called with a non-sub-track"
        );

        // Show the port label (after "→") rather than the full name —
        // "Drums → Kick" becomes "Kick", which fits comfortably inside
        // the narrower strip.
        let short_name = track.name.split(" \u{2192} ").nth(1).unwrap_or(&track.name);
        let display_name = crate::util::short(short_name, 10);
        let name_text = container(
            text(display_name)
                .size(11)
                .font(theme::UI_FONT_MEDIUM)
                .color(theme::TEXT_2)
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .width(Length::Fill)
        .clip(true);

        let head_row = row![name_text]
            .spacing(0)
            .align_y(alignment::Vertical::Center)
            .height(28);
        let head: Element<'_, Message> = container(head_row)
            .width(Length::Fill)
            .padding([6, 8])
            .into();

        // M / S / FX-bypass — the three controls that actually apply to
        // a plugin-fed sub-track. Record-arm and monitor are intentionally
        // omitted: sub-tracks have no input.
        let button_row = container(
            row![
                mute_button(
                    track.muted,
                    Message::Track(TrackMessage::ToggleMute(track.id)),
                    11
                ),
                solo_button(
                    track.soloed,
                    Message::Track(TrackMessage::ToggleSolo(track.id)),
                    11
                ),
                fx_bypass_button(
                    track.fx_bypassed,
                    Message::Track(TrackMessage::ToggleTrackFxBypass(track.id)),
                    11,
                ),
            ]
            .spacing(2)
            .align_y(alignment::Vertical::Center),
        )
        .width(Length::Fill)
        .center_x(Length::Fill);

        let id = track.id;
        let pan_ctrl = pan_knob(track.pan, move |v| {
            Message::Track(TrackMessage::SetTrackPan(id, v))
        });
        let pan_label = text(crate::util::format_pan(track.pan))
            .size(9)
            .font(Font::MONOSPACE)
            .color(theme::TEXT_DIM);
        let pan_block = column![
            container(pan_ctrl).width(Length::Fill).center_x(Length::Fill),
            container(pan_label).width(Length::Fill).center_x(Length::Fill),
        ]
        .spacing(2)
        .align_x(alignment::Horizontal::Center);

        let track_id_for_fader = track.id;
        let fader_block = fader_section(track.level_l, track.level_r, track.volume, move |v| {
            Message::Track(TrackMessage::SetTrackVolume(track_id_for_fader, v))
        });

        let is_selected = self.interaction.selected_track == Some(track.id);

        // Left-edge accent rail. A thin colored column the full strip
        // height — sits flush against the left edge so the eye reads a
        // visual tie to the parent strip on its left. Saturates to the
        // full lavender when the sub-track is selected.
        let rail_color = if is_selected {
            theme::ACCENT
        } else {
            theme::MIXER_SUB_STRIP_RAIL
        };
        let rail = container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fixed(theme::MIXER_SUB_STRIP_RAIL_WIDTH))
            .height(Length::Fill)
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(rail_color)),
                ..Default::default()
            });

        // Body content sits to the right of the rail. Spacer pads the
        // top a bit so the head label aligns roughly with the parent
        // strip's name row.
        let body = column![
            head,
            Space::new().height(6),
            button_row,
            Space::new().height(8),
            pan_block,
            Space::new().height(Length::Fill),
            fader_block,
            Space::new().height(6),
        ]
        .spacing(0)
        .width(Length::Fill)
        .height(Length::Fill);

        let border_color = if is_selected {
            theme::ACCENT_LINE
        } else {
            theme::LINE_2
        };
        let border_w = if is_selected { 1.0 } else { 0.5 };
        let strip_style = move |_theme: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(theme::MIXER_SUB_STRIP_BG)),
            border: iced::Border {
                color: border_color,
                width: border_w,
                // Slightly tighter corner radius than the parent strip
                // so the recessed shape reads as nested rather than as
                // a peer card.
                radius: theme::RADIUS_LG.into(),
            },
            ..Default::default()
        };

        let inner = row![rail, body]
            .spacing(0)
            .height(Length::Fill)
            .width(Length::Fill);

        let track_id_for_select = track.id;
        let strip_height = Length::Fixed(theme::MIXER_STRIP_HEIGHT as f32);
        mouse_area(
            container(inner)
                .width(Length::Fixed(theme::MIXER_SUB_STRIP_WIDTH))
                .height(strip_height)
                .style(strip_style),
        )
        .on_press(Message::Ui(UiMessage::SelectTrack(Some(track_id_for_select))))
        .into()
    }

    /// Compact subtrack meters shown in the right half of a collapsed
    /// parent strip. Each subtrack gets a name label and stereo meter bars.
    fn view_collapsed_subtrack_meters(&self, parent_id: TrackId) -> Element<'_, Message> {
        let mut subtracks: Vec<&TrackState> = self
            .registry
            .tracks
            .iter()
            .filter(|t| matches!(t.sub_track, Some(link) if link.parent_track_id == parent_id))
            .collect();
        subtracks.sort_by_key(|t| t.order);

        // Each meter column gets a fixed width so labels never wrap into
        // their glyph cells. 28px fits "Snare" / "Hi-Hat" abbreviated to
        // four characters at size 9.
        const COL_W: u16 = 28;
        let mut meters_row = row![].spacing(2);
        for sub in &subtracks {
            // Show the port label (after "→") rather than the full name,
            // so "Instrument 2 → Kick" displays as "Kick" instead of "Inst.".
            let short_name = sub.name.split(" \u{2192} ").nth(1).unwrap_or(&sub.name);
            let label = crate::util::short(short_name, 5);
            let name_label = text(label)
                .size(9)
                .color(theme::TEXT_3)
                .wrapping(iced::widget::text::Wrapping::None);
            let meter = meter_v(sub.level_l, sub.level_r, theme::FADER_HEIGHT);
            let col = column![
                container(name_label)
                    .width(Length::Fill)
                    .center_x(Length::Fill)
                    .clip(true),
                container(meter).width(Length::Fill).center_x(Length::Fill),
            ]
            .spacing(2)
            .width(Length::Fixed(COL_W as f32))
            .align_x(alignment::Horizontal::Center);
            meters_row = meters_row.push(col);
        }

        let title = text(format!("{} outs", subtracks.len()))
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3);

        // The meters sit at the bottom of the column so they line up with
        // the parent strip's fader. Bottom offset matches the fader-label
        // band so both columns end on the same baseline.
        column![
            title,
            Space::new().height(Length::Fill),
            meters_row,
            Space::new().height(20),
        ]
        .spacing(2)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

}

/// Strip head background — bottom hairline that separates the head from
/// the rest of the strip card.
fn strip_head_bg(_theme: &iced::Theme) -> container::Style {
    container::Style {
        border: iced::Border {
            color: theme::LINE_2,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}
