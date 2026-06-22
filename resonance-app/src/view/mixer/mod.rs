//! Mixer view: top-level layout, the small "+ Bus" strip, and the
//! shared `view_plugin_slot_row` helper used by every channel strip.
//! The actual strip rendering lives in submodules — `track_strip.rs`,
//! `bus_strip.rs`, `master_strip.rs`, `plugin_panel.rs`.

mod bus_strip;
mod inspector;
mod master_strip;
pub(crate) mod picks;
mod plugin_panel;
mod reference_panel;
mod track_strip;

use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::message::*;
use crate::state::*;
use crate::theme;

use picks::PluginOwner;

impl crate::Resonance {
    pub(crate) fn view_mixer(&self) -> Element<'_, Message> {
        let sorted_tracks = self.sorted_tracks();
        let sorted_busses = self.sorted_busses();
        let available_plugins = &self.available_plugins;

        // -- Top row: track strips + master strip on the right. --
        // Sub-tracks render as their own strips next to the parent. The
        // previous pass walked `sorted_tracks` linearly, which placed
        // sub-tracks at their `.order` position — wherever they happened
        // to be allocated, often after several unrelated tracks. That
        // broke the parent → child relationship at a glance.
        //
        // New pass: for every top-level track, emit the parent strip
        // and immediately follow it with its sub-tracks (in
        // `output_port_index` order — the same order the engine fans
        // them out). Sub-tracks are skipped in the outer iteration so
        // they only appear right after their parent. Each parent + its
        // sub-tracks is wrapped in a tight inner row (0 px spacing) so
        // the cluster visually attaches; the outer row keeps
        // `MIXER_STRIP_GAP` between unrelated tracks, and the lane gets
        // a `MIXER_LANE_HPAD` lead-in so the first strip doesn't sit
        // flush against the window edge.
        let mut track_strip_row = row![]
            .spacing(theme::MIXER_STRIP_GAP)
            .padding([0.0, theme::MIXER_LANE_HPAD]);
        for track in sorted_tracks {
            if track.sub_track.is_some() {
                // Already emitted by its parent's cluster (or skipped
                // because its parent was collapsed).
                continue;
            }

            let parent_strip = self.view_channel_strip(track, available_plugins);

            let parent_expanded = self.mixer.expanded_sub_track_parents.contains(&track.id);
            let mut subs: Vec<&TrackState> = if parent_expanded {
                sorted_tracks
                    .iter()
                    .filter(|t| {
                        matches!(t.sub_track, Some(link) if link.parent_track_id == track.id)
                    })
                    .collect()
            } else {
                Vec::new()
            };
            // Sort sub-strips by their plugin output port index, not by
            // `.order`. The port index is stable across project
            // load/save; `.order` is the allocation-time counter and
            // can interleave with unrelated tracks. With a stable port
            // sort, "Kick / Snare / HH / Tom" always renders in the
            // same left-to-right order regardless of when each
            // sub-track was created.
            subs.sort_by_key(|t| {
                t.sub_track
                    .map(|l| l.output_port_index)
                    .unwrap_or(0)
            });

            if subs.is_empty() {
                track_strip_row = track_strip_row.push(parent_strip);
            } else {
                // Cluster: parent + sub-strips with no internal gap, so
                // the recessed sub-strip backgrounds visually butt up
                // against the parent strip.
                let mut cluster = row![parent_strip].spacing(0);
                for sub in subs {
                    cluster =
                        cluster.push(self.view_sub_channel_strip(sub, available_plugins));
                }
                track_strip_row = track_strip_row.push(cluster);
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

    /// Render a single plugin slot row (name button + remove button).
    /// If `is_instrument_slot` is true, the name is tinted to distinguish it.
    fn view_plugin_slot_row(
        &self,
        owner: PluginOwner,
        plugin: &PluginSlotState,
        is_instrument_slot: bool,
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

        // Button takes Length::Fill so it stretches to the strip width;
        // the delete button hugs the right edge.
        row![name_btn, plugin_del]
            .spacing(2)
            .align_y(alignment::Vertical::Center)
            .into()
    }
}
