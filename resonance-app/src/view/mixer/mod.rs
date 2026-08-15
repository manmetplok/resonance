//! Mixer view: top-level layout, the small "+ Bus" strip, and the
//! shared `view_plugin_slot_row` helper used by every channel strip.
//! The actual strip rendering lives in submodules — `track_strip.rs`,
//! `bus_strip.rs`, `master_strip.rs`, `plugin_panel.rs`.

pub(crate) mod automation;
mod bus_strip;
mod group_strip;
pub(crate) mod inspector;
mod master_strip;
pub(crate) mod picks;
mod plugin_panel;
mod reference_panel;
pub(crate) mod reorder;
mod track_strip;

use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{alignment, Color, Element, Length};

use resonance_audio::types::ScannedPlugin;

use crate::message::*;
use crate::state::*;
use crate::theme;

use picks::PluginOwner;

pub(crate) use group_strip::MixerTopItem;

impl crate::Resonance {
    pub(crate) fn view_mixer(&self) -> Element<'_, Message> {
        let sorted_tracks = self.sorted_tracks();
        let sorted_busses = self.sorted_busses();
        let available_plugins = &self.available_plugins;

        // -- Top row: track strips + master strip on the right. --
        // The lane is built in two clustering layers so related strips
        // stay visually attached instead of scattering to their `.order`
        // positions:
        //   1. Top-level order (`mixer_top_level_items`): ungrouped
        //      tracks keep their sorted position; a track group folds
        //      into one coloured cluster at the slot of its first member
        //      (`view_mixer_group_cluster`).
        //   2. Per track (`view_track_cluster`): a parent strip is
        //      immediately followed by its expanded sub-tracks (in
        //      `output_port_index` order — how the engine fans them out)
        //      in a tight inner row (0 px spacing).
        // The outer row keeps `MIXER_STRIP_GAP` between unrelated
        // strips/clusters, and the lane gets a `MIXER_LANE_HPAD` lead-in
        // so the first strip doesn't sit flush against the window edge.
        let mut track_strip_row = row![]
            .spacing(theme::MIXER_STRIP_GAP)
            .padding([0.0, theme::MIXER_LANE_HPAD]);
        for item in self.mixer_top_level_items() {
            match item {
                group_strip::MixerTopItem::Track(track_id) => {
                    if let Some(track) = sorted_tracks.iter().find(|t| t.id == track_id) {
                        track_strip_row = track_strip_row.push(self.view_track_cluster(
                            track,
                            sorted_tracks,
                            available_plugins,
                        ));
                    }
                }
                group_strip::MixerTopItem::Group(group_id) => {
                    if let Some(group) = self.track_groups.get_group(group_id) {
                        track_strip_row = track_strip_row.push(self.view_mixer_group_cluster(
                            group,
                            sorted_tracks,
                            available_plugins,
                        ));
                    }
                }
            }
        }
        // Construct the scrollable with its horizontal direction up
        // front. `scrollable(content)` would default to Vertical and run
        // its `validate()` debug assertion before the chained
        // `.direction(...)` has a chance to change it — and
        // `track_strip_row`'s size hint is Fill-height now that each
        // strip claims Length::Fill vertically.
        let scrollable_tracks = iced::widget::Scrollable::with_direction(
            track_strip_row,
            scrollable::Direction::Horizontal(scrollable::Scrollbar::default()),
        )
        .width(Length::Fill);
        let master_strip = self.view_master_strip(available_plugins);
        let v_separator_tracks = container(Space::new().width(1).height(Length::Fill)).style(theme::separator_bg);
        let tracks_area = row![scrollable_tracks, v_separator_tracks, master_strip]
            .height(Length::Fixed(theme::MIXER_STRIP_HEIGHT as f32));

        // -- Bottom row: bus strips + "+ Bus" button on the right. --
        let mut bus_strip_row = row![]
            .spacing(theme::MIXER_STRIP_GAP)
            .padding([0.0, theme::MIXER_LANE_HPAD]);
        for bus in sorted_busses {
            bus_strip_row = bus_strip_row.push(self.view_bus_strip(bus, available_plugins));
        }
        let scrollable_busses = iced::widget::Scrollable::with_direction(
            bus_strip_row,
            scrollable::Direction::Horizontal(scrollable::Scrollbar::default()),
        )
        .width(Length::Fill);
        let add_bus_strip = self.view_add_bus_strip();
        let v_separator_busses = container(Space::new().width(1).height(Length::Fill)).style(theme::separator_bg);
        let busses_area = row![scrollable_busses, v_separator_busses, add_bus_strip]
            .height(Length::Fixed(theme::BUS_STRIP_HEIGHT as f32));

        let h_sep_mid = container(Space::new().width(Length::Fill).height(1)).style(theme::separator_bg);

        let mut mixer_col = column![].spacing(0);
        mixer_col = mixer_col.push(tracks_area);
        mixer_col = mixer_col.push(h_sep_mid);
        mixer_col = mixer_col.push(busses_area);

        if let Some(panel) = self.view_plugin_panel() {
            let h_sep = container(Space::new().width(Length::Fill).height(1)).style(theme::separator_bg);
            mixer_col = mixer_col.push(h_sep);
            mixer_col = mixer_col.push(panel);
        }

        // Inspector sits to the right of the strips; a hairline separates
        // it from the strips column.
        let inspector_panel = inspector::view(self);
        let v_sep_inspector =
            container(Space::new().width(1).height(Length::Fill)).style(theme::separator_bg);

        let mut body = row![
            container(mixer_col).width(Length::Fill).height(Length::Fill),
            v_sep_inspector,
            inspector_panel,
        ]
        .height(Length::Fill);

        // The Reference & A/B rail is the outermost right rail, shown only
        // when the chrome "REF" toggle is on. A hairline separates it from
        // the inspector.
        if self.mixer.reference_panel_open {
            let v_sep_reference =
                container(Space::new().width(1).height(Length::Fill)).style(theme::separator_bg);
            body = body
                .push(v_sep_reference)
                .push(reference_panel::view(self));
        }

        container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::base_bg)
            .into()
    }

    /// Render a top-level track strip plus its expanded sub-track strips as
    /// a tight (0 px) cluster — the parent + sub-track grouping lifted out
    /// of `view_mixer` so it is reused both standalone and inside a group
    /// cluster (`view_mixer_group_cluster`). Sub-tracks sort by their plugin
    /// `output_port_index` (stable across save/load) and render through the
    /// recessed `view_sub_channel_strip`; an unexpanded or sub-less parent
    /// just returns its own strip.
    pub(super) fn view_track_cluster<'a>(
        &'a self,
        track: &'a TrackState,
        sorted_tracks: &'a [TrackState],
        available_plugins: &'a [ScannedPlugin],
    ) -> Element<'a, Message> {
        let parent_strip = self.view_channel_strip(track, available_plugins);

        let parent_expanded = self.mixer.expanded_sub_track_parents.contains(&track.id);
        if !parent_expanded {
            return parent_strip;
        }
        let mut subs: Vec<&TrackState> = sorted_tracks
            .iter()
            .filter(|t| matches!(t.sub_track, Some(link) if link.parent_track_id == track.id))
            .collect();
        if subs.is_empty() {
            return parent_strip;
        }
        subs.sort_by_key(|t| t.sub_track.map(|l| l.output_port_index).unwrap_or(0));

        // Cluster: parent + sub-strips with no internal gap, so the recessed
        // sub-strip backgrounds visually butt up against the parent strip.
        let mut cluster = row![parent_strip].spacing(0);
        for sub in subs {
            cluster = cluster.push(self.view_sub_channel_strip(sub, available_plugins));
        }
        cluster.into()
    }

    /// Small "+ Bus" strip that lives in the same slot the master strip
    /// occupies in the top row, but in the bus row. Clicking it dispatches
    /// `Message::Bus(BusMessage::AddBus)`.
    fn view_add_bus_strip(&self) -> Element<'_, Message> {
        let label = container(text("Busses").size(11).color(theme::TEXT_DIM))
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding([6, 4]);

        let add_btn = button(text("+ Bus").size(11).color(theme::TEXT))
            .on_press(Message::Bus(BusMessage::AddBus))
            .style(|_theme, status| theme::small_button_style(status))
            .padding([4, 10]);

        let content = column![
            label,
            container(add_btn)
                .width(Length::Fill)
                .center_x(Length::Fill),
            Space::new().height(Length::Fill),
        ]
        .spacing(4)
        .padding(8)
        .width(theme::MASTER_STRIP_WIDTH);

        container(content)
            .height(Length::Fill)
            .style(theme::panel_dark_outlined)
            .into()
    }

    /// Render a single plugin slot row (name button + ▲/▼ reorder pair +
    /// remove button). If `is_instrument_slot` is true, the name is
    /// tinted to distinguish it.
    ///
    /// `index` is the slot's position in `owner`'s chain and `len` the
    /// chain's length — the two the reorder controls need to know which
    /// direction is still available (ba todo #1302). They are the
    /// position in the FULL chain, not in the section being drawn: the
    /// track strip renders the instrument and the effects as two
    /// sections, and the instrument-floor rule is stated in chain
    /// indices.
    fn view_plugin_slot_row(
        &self,
        owner: PluginOwner,
        plugin: &PluginSlotState,
        is_instrument_slot: bool,
        index: usize,
        len: usize,
    ) -> Element<'_, Message> {
        // ASCII ".." suffix (not '…') — this pill's width was tuned
        // around the narrower two-dot tail.
        let pname = crate::util::short_with(&plugin.plugin_name, 14, "..");
        let pid = plugin.instance_id;
        // Plugins that expose a floating editor (has_gui) are driven
        // entirely from that window — clicking the name in the strip
        // toggles the editor open/closed rather than showing the
        // generic params in the bottom panel. Plugins without a
        // floating editor still fall back to the bottom panel path.
        let (click_msg, is_selected) = if plugin.has_gui {
            let msg = if plugin.editor_open {
                Message::Plugin(PluginMessage::ClosePluginEditor(pid))
            } else {
                Message::Plugin(PluginMessage::OpenPluginEditor(pid))
            };
            (msg, plugin.editor_open)
        } else {
            (
                Message::Plugin(PluginMessage::TogglePluginPanel(pid)),
                self.mixer.selected_plugin == Some(pid),
            )
        };

        // Instrument slots get the design's lavender pill: ◆ glyph
        // followed by the plugin name on a tinted ACCENT_DIM background
        // with an ACCENT_LINE border. FX slots stay as a plainer hairline
        // pill so the eye picks up the instrument as the dominant slot.
        let name_btn = if is_instrument_slot {
            let label_color = if is_selected {
                theme::TEXT_1
            } else {
                theme::ACCENT_SOFT
            };
            let pill = row![
                text("\u{25C6}").size(8).color(theme::ACCENT_SOFT),
                Space::new().width(6),
                text(pname).size(10).color(label_color),
            ]
            .align_y(alignment::Vertical::Center);
            button(pill)
                .on_press(click_msg)
                .width(Length::Fill)
                .style(move |_theme, status| {
                    let bg = match status {
                        iced::widget::button::Status::Hovered => Color {
                            a: 0.22,
                            ..theme::ACCENT
                        },
                        iced::widget::button::Status::Pressed => Color {
                            a: 0.30,
                            ..theme::ACCENT
                        },
                        _ => theme::ACCENT_DIM,
                    };
                    iced::widget::button::Style {
                        background: Some(iced::Background::Color(bg)),
                        text_color: theme::ACCENT_SOFT,
                        border: iced::Border {
                            color: theme::ACCENT_LINE,
                            width: 1.0,
                            radius: theme::RADIUS_SM.into(),
                        },
                        ..Default::default()
                    }
                })
                .padding([7, 9])
        } else {
            let label_color = if is_selected {
                theme::TEXT_1
            } else {
                theme::TEXT_2
            };
            button(text(pname).size(10).color(label_color))
                .on_press(click_msg)
                .width(Length::Fill)
                .style(move |_theme, status| {
                    let bg = match status {
                        iced::widget::button::Status::Hovered => theme::BG_3,
                        iced::widget::button::Status::Pressed => theme::LINE_2,
                        _ => theme::BG_1,
                    };
                    let border_color = if is_selected {
                        theme::ACCENT_LINE
                    } else {
                        theme::LINE_2
                    };
                    iced::widget::button::Style {
                        background: Some(iced::Background::Color(bg)),
                        text_color: label_color,
                        border: iced::Border {
                            color: border_color,
                            width: 1.0,
                            radius: theme::RADIUS_SM.into(),
                        },
                        ..Default::default()
                    }
                })
                .padding([5, 9])
        };

        let remove_msg = match owner {
            PluginOwner::Track(track_id) => {
                Message::Plugin(PluginMessage::RemovePluginFromTrack(track_id, pid))
            }
            PluginOwner::Bus(bus_id) => Message::Bus(BusMessage::RemovePluginFromBus(bus_id, pid)),
            PluginOwner::Master => Message::Master(MasterMessage::RemovePluginFromMaster(pid)),
        };
        let plugin_del = button(text("\u{00d7}").size(9).color(theme::TEXT_DIM))
            .on_press(remove_msg)
            .style(|_theme, status| theme::small_button_style(status))
            .padding(1);

        // Chain reorder (ba todo #1302). The strip is 140 px wide, so
        // the carets are drawn at the same 9 px as the delete glyph and
        // sit between the name and the ×: order first, then removal.
        let moves = reorder::chain_moves(self, owner, pid, index, len);

        // Button takes Length::Fill so it stretches to the strip width;
        // the reorder pair and the delete button hug the right edge.
        row![name_btn, reorder::move_buttons(&moves, 9.0), plugin_del]
            .spacing(2)
            .align_y(alignment::Vertical::Center)
            .into()
    }
}
