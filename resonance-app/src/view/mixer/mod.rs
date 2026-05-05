//! Mixer view: top-level layout, the small "+ Bus" strip, and the
//! shared `view_plugin_slot_row` helper used by every channel strip.
//! The actual strip rendering lives in submodules — `track_strip.rs`,
//! `bus_strip.rs`, `master_strip.rs`, `plugin_panel.rs`.

mod bus_strip;
mod master_strip;
mod picks;
mod plugin_panel;
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
        // Skip sub-tracks whose parent is currently collapsed in the
        // mixer UI. The collapse state is app-side only (not persisted).
        let mut track_strip_row = row![].spacing(2);
        for track in &sorted_tracks {
            if let Some(link) = track.sub_track {
                if !self
                    .mixer
                    .expanded_sub_track_parents
                    .contains(&link.parent_track_id)
                {
                    continue;
                }
            }
            track_strip_row =
                track_strip_row.push(self.view_channel_strip(track, available_plugins));
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
        let v_separator_tracks = container(Space::new(1, Length::Fill)).style(theme::separator_bg);
        let tracks_area = row![scrollable_tracks, v_separator_tracks, master_strip]
            .height(Length::FillPortion(1));

        // -- Bottom row: bus strips + "+ Bus" button on the right. --
        let mut bus_strip_row = row![].spacing(2);
        for bus in &sorted_busses {
            bus_strip_row = bus_strip_row.push(self.view_bus_strip(bus, available_plugins));
        }
        let scrollable_busses = iced::widget::Scrollable::with_direction(
            bus_strip_row,
            scrollable::Direction::Horizontal(scrollable::Scrollbar::default()),
        )
        .width(Length::Fill);
        let add_bus_strip = self.view_add_bus_strip();
        let v_separator_busses = container(Space::new(1, Length::Fill)).style(theme::separator_bg);
        let busses_area = row![scrollable_busses, v_separator_busses, add_bus_strip]
            .height(Length::FillPortion(1));

        let h_sep_mid = container(Space::new(Length::Fill, 1)).style(theme::separator_bg);

        let mut mixer_col = column![].spacing(0);
        mixer_col = mixer_col.push(tracks_area);
        mixer_col = mixer_col.push(h_sep_mid);
        mixer_col = mixer_col.push(busses_area);

        if let Some(panel) = self.view_plugin_panel() {
            let h_sep = container(Space::new(Length::Fill, 1)).style(theme::separator_bg);
            mixer_col = mixer_col.push(h_sep);
            mixer_col = mixer_col.push(panel);
        }

        container(mixer_col)
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
            Space::with_height(Length::Fill),
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
        let pname: String = if plugin.plugin_name.chars().count() > 14 {
            let mut s: String = plugin.plugin_name.chars().take(12).collect();
            s.push_str("..");
            s
        } else {
            plugin.plugin_name.clone()
        };
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

        let base_color = if is_instrument_slot {
            Color::from_rgb(0.3, 0.75, 0.8)
        } else {
            theme::ACCENT
        };
        let name_color = if is_selected { theme::TEXT } else { base_color };
        let name_btn = button(text(pname).size(9).color(name_color))
            .on_press(click_msg)
            .style(move |_theme, status| {
                if is_selected {
                    let bg = match status {
                        iced::widget::button::Status::Hovered => Color::from_rgb(0.22, 0.22, 0.28),
                        iced::widget::button::Status::Pressed => Color::from_rgb(0.15, 0.15, 0.20),
                        _ => Color::from_rgb(0.18, 0.18, 0.24),
                    };
                    iced::widget::button::Style {
                        background: Some(iced::Background::Color(bg)),
                        text_color: theme::TEXT,
                        border: iced::Border {
                            color: base_color,
                            width: 1.0,
                            radius: 2.0.into(),
                        },
                        ..Default::default()
                    }
                } else {
                    theme::small_button_style(status)
                }
            })
            .padding(1);

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

        row![name_btn, Space::with_width(Length::Fill), plugin_del,]
            .spacing(2)
            .align_y(alignment::Vertical::Center)
            .into()
    }
}
