/// Mixer view rendering for the Resonance application.
use crate::message::Message;
use crate::state::*;
use crate::theme;
use crate::util::{format_db, format_pan};
use iced::widget::{
    button, column, container, pick_list, row, scrollable, slider, text,
    vertical_slider, Space,
};
use iced::{alignment, Color, Element, Font, Length};
use resonance_audio::types::*;

/// Convert linear amplitude to meter bar height (logarithmic/dB scale).
fn level_to_bar_height(level: f32, max_height: f32) -> f32 {
    if level < 0.0001 {
        return 0.0;
    }
    let db = 20.0 * level.log10();
    let normalized = (db + 60.0) / 66.0; // -60dB=0, +6dB=1
    normalized.clamp(0.0, 1.0) * max_height
}

/// Get meter color based on signal level (green / yellow / red).
fn level_color(level: f32) -> Color {
    if level < 0.0001 {
        return theme::METRONOME_ON;
    }
    let db = 20.0 * level.log10();
    if db > 0.0 {
        theme::RECORD_RED
    } else if db > -6.0 {
        theme::SOLO_YELLOW
    } else {
        theme::METRONOME_ON
    }
}

/// Render a single vertical VU meter bar (bottom-aligned colored bar on dark bg).
fn meter_bar_v<'a>(level: f32, max_height: f32) -> Element<'a, Message> {
    let bar_height = level_to_bar_height(level, max_height);
    let color = level_color(level);

    // Spacer pushes the colored bar to the bottom
    let spacer_height = (max_height - bar_height).max(0.0);

    let bar = container(Space::new(0.0, 0.0))
        .width(Length::Fill)
        .height(bar_height)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(color)),
            ..Default::default()
        });

    container(
        column![
            Space::new(Length::Fill, spacer_height),
            bar,
        ],
    )
    .width(6)
    .height(max_height)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::METER_BG)),
        ..Default::default()
    })
    .into()
}

/// Render a stereo vertical VU meter (L + R bars side by side).
fn view_meter_v<'a>(level_l: f32, level_r: f32, height: f32) -> Element<'a, Message> {
    row![meter_bar_v(level_l, height), meter_bar_v(level_r, height)]
        .spacing(1)
        .into()
}

impl crate::Resonance {
    pub(crate) fn view_mixer(&self) -> Element<'_, Message> {
        let sorted_tracks = self.sorted_tracks();

        let available_plugins = self.available_plugins.clone();
        let mut strip_row = row![].spacing(2);
        for track in &sorted_tracks {
            strip_row = strip_row.push(self.view_channel_strip(track, &available_plugins));
        }

        let scrollable_strips = scrollable(strip_row)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default(),
            ))
            .width(Length::Fill);

        let master_strip = self.view_master_strip();

        let separator = container(Space::new(1, Length::Fill)).style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::SEPARATOR)),
            ..Default::default()
        });

        let strips_area = row![scrollable_strips, separator, master_strip]
            .height(Length::Fill);

        // Bottom plugin panel
        let mut mixer_col = column![].spacing(0);
        mixer_col = mixer_col.push(strips_area);

        if let Some(panel) = self.view_plugin_panel() {
            let h_sep = container(Space::new(Length::Fill, 1)).style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::SEPARATOR)),
                ..Default::default()
            });
            mixer_col = mixer_col.push(h_sep);
            mixer_col = mixer_col.push(panel);
        }

        container(mixer_col)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::BG)),
                ..Default::default()
            })
            .into()
    }

    fn view_channel_strip(&self, track: &TrackState, available_plugins: &[ScannedPlugin]) -> Element<'_, Message> {
        let track_name = container(
            text(track.name.clone()).size(13).color(theme::TEXT),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .padding([6, 4]);

        // Mute / Solo / Arm / Monitor buttons
        let rec_color = if track.record_armed { theme::RECORD_RED } else { theme::TEXT_DIM };
        let armed = track.record_armed;
        let rec_btn = button(text("R").size(11).color(rec_color))
            .on_press(Message::ToggleRecordArm(track.id))
            .style(move |_theme, status| {
                if armed { theme::record_armed_button_style(status) }
                else { theme::small_button_style(status) }
            })
            .padding(2);

        let mute_color = if track.muted { theme::ACCENT } else { theme::TEXT_DIM };
        let mute_btn = button(text("M").size(11).color(mute_color))
            .on_press(Message::ToggleMute(track.id))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2);

        let solo_color = if track.soloed { theme::SOLO_YELLOW } else { theme::TEXT_DIM };
        let solo_btn = button(text("S").size(11).color(solo_color))
            .on_press(Message::ToggleSolo(track.id))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2);

        let mon_color = if track.monitor_enabled { theme::METRONOME_ON } else { theme::TEXT_DIM };
        let mon_enabled = track.monitor_enabled;
        let mon_btn = button(text("I").size(11).color(mon_color))
            .on_press(Message::ToggleMonitor(track.id))
            .style(move |_theme, status| {
                theme::toggle_button_style(mon_enabled, theme::METRONOME_ON, true, status)
            })
            .padding(2);

        let is_mono = track.mono;
        let mono_label = if track.mono { "M" } else { "S" };
        let mono_btn = button(text(mono_label).size(11).color(theme::TEXT))
            .on_press(Message::ToggleTrackMono(track.id))
            .style(move |_theme, status| {
                theme::mono_button_style(is_mono, status)
            })
            .padding(2);

        let button_row = row![mono_btn, mon_btn, rec_btn, mute_btn, solo_btn]
            .spacing(4)
            .align_y(alignment::Vertical::Center);

        // Plugin chain (click to show in bottom panel)
        let mut plugin_section = column![].spacing(2).width(Length::Fill);
        for plugin in &track.plugins {
            let pname: String = if plugin.plugin_name.chars().count() > 14 {
                let mut s: String = plugin.plugin_name.chars().take(12).collect();
                s.push_str("..");
                s
            } else {
                plugin.plugin_name.clone()
            };
            let track_id = track.id;
            let pid = plugin.instance_id;
            let is_selected = self.selected_plugin == Some(pid);

            let name_color = if is_selected { theme::TEXT } else { theme::ACCENT };
            let name_btn = button(text(pname).size(9).color(name_color))
                .on_press(Message::TogglePluginPanel(pid))
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
                                color: theme::ACCENT,
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

            let plugin_del = button(text("\u{00d7}").size(9).color(theme::TEXT_DIM))
                .on_press(Message::RemovePluginFromTrack(track_id, pid))
                .style(|_theme, status| theme::small_button_style(status))
                .padding(1);

            let plugin_row = row![
                name_btn,
                Space::with_width(Length::Fill),
                plugin_del,
            ]
            .spacing(2)
            .align_y(alignment::Vertical::Center);
            plugin_section = plugin_section.push(plugin_row);
        }

        // FX picker
        if !available_plugins.is_empty() {
            let track_id = track.id;
            let fx_picker = pick_list(
                available_plugins.to_vec(),
                None::<ScannedPlugin>,
                move |plugin: ScannedPlugin| Message::AddPluginToTrack(track_id, plugin),
            )
            .placeholder("+ FX")
            .text_size(10)
            .width(Length::Fill);
            plugin_section = plugin_section.push(fx_picker);
        }

        // Pan control (horizontal)
        let pan_slider = slider(-1.0..=1.0f32, track.pan, {
            let id = track.id;
            move |v| Message::SetTrackPan(id, v)
        })
        .width(Length::Fill)
        .step(0.01);

        let pan_label = format_pan(track.pan);
        let pan_row = row![
            text("Pan").size(9).color(theme::TEXT_DIM),
            Space::with_width(4),
            pan_slider,
            Space::with_width(4),
            text(pan_label).size(9).font(Font::MONOSPACE).color(theme::TEXT_DIM),
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center);

        // Volume fader (vertical) + VU meters
        let fader_height = 120.0;

        let vol_fader = vertical_slider(-60.0..=6.0f32, track.volume, {
            let id = track.id;
            move |v| Message::SetTrackVolume(id, v)
        })
        .height(fader_height)
        .step(0.1);

        let meters = view_meter_v(track.level_l, track.level_r, fader_height);

        let vol_label = format_db(track.volume);
        let fader_row = row![
            meters,
            vol_fader,
        ]
        .spacing(4)
        .align_y(alignment::Vertical::Center);

        let fader_section = column![
            container(fader_row)
                .width(Length::Fill)
                .center_x(Length::Fill),
            text(vol_label).size(9).font(Font::MONOSPACE).color(theme::TEXT_DIM),
        ]
        .spacing(2)
        .align_x(alignment::Horizontal::Center);

        // Input device picker (when armed)
        let mut bottom_section = column![].spacing(2);
        if track.record_armed && !self.input_devices.is_empty() {
            let selected = track
                .input_device_name
                .as_ref()
                .and_then(|name| self.input_devices.iter().find(|d| &d.name == name))
                .cloned();

            let track_id = track.id;
            let device_picker = pick_list(
                self.input_devices.clone(),
                selected,
                move |device: InputDeviceInfo| {
                    Message::SetTrackInputDevice(track_id, Some(device.name))
                },
            )
            .placeholder("Select input...")
            .text_size(10)
            .width(Length::Fill);

            bottom_section = bottom_section.push(device_picker);
        }

        let bg = if track.record_armed { theme::PANEL_ARMED } else { theme::PANEL_DARK };
        let border_color = if track.record_armed { theme::RECORD_RED } else { theme::SEPARATOR };

        let strip_content = column![
            track_name,
            button_row,
            plugin_section,
            pan_row,
            fader_section,
            bottom_section,
        ]
        .spacing(4)
        .padding(6)
        .width(theme::MIXER_STRIP_WIDTH);

        container(strip_content)
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

    fn view_master_strip(&self) -> Element<'_, Message> {
        let label = container(
            text("Master").size(14).color(theme::ACCENT),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .padding([6, 4]);

        let fader_height = 120.0;

        let vol_fader = vertical_slider(-60.0..=6.0f32, self.master_volume, Message::SetMasterVolume)
            .height(fader_height)
            .step(0.1);

        let meters = view_meter_v(self.master_level_l, self.master_level_r, fader_height);

        let vol_label = format_db(self.master_volume);

        let fader_row = row![
            meters,
            vol_fader,
        ]
        .spacing(4)
        .align_y(alignment::Vertical::Center);

        let fader_section = column![
            container(fader_row)
                .width(Length::Fill)
                .center_x(Length::Fill),
            text(vol_label).size(9).font(Font::MONOSPACE).color(theme::TEXT_DIM),
        ]
        .spacing(2)
        .align_x(alignment::Horizontal::Center);

        let bounce_btn: Element<'_, Message> = if self.bouncing {
            text("Bouncing...").size(8).color(theme::ACCENT).into()
        } else {
            button(text("Bounce").size(8).color(theme::TEXT))
                .on_press(Message::BounceToWav)
                .style(|_theme, status| theme::small_button_style(status))
                .padding([2, 8])
                .into()
        };

        let bounce_row = container(bounce_btn)
            .width(Length::Fill)
            .center_x(Length::Fill);

        let strip_content = column![
            label,
            bounce_row,
            Space::with_height(Length::Fill),
            fader_section,
        ]
        .spacing(4)
        .padding(8)
        .width(theme::MASTER_STRIP_WIDTH);

        container(strip_content)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::PANEL_DARK)),
                border: iced::Border {
                    color: theme::SEPARATOR,
                    width: 0.5,
                    radius: 0.0.into(),
                },
                ..Default::default()
            })
            .into()
    }

    /// Bottom panel showing the selected plugin's UI.
    fn view_plugin_panel(&self) -> Option<Element<'_, Message>> {
        let selected_id = self.selected_plugin?;

        // Find the plugin across all tracks
        let plugin = self.tracks.iter()
            .flat_map(|t| t.plugins.iter())
            .find(|p| p.instance_id == selected_id)?;

        let ui_params: Vec<resonance_plugin::ui::UiParam> = plugin.params.iter()
            .map(|p| resonance_plugin::ui::UiParam {
                id: p.id,
                name: p.name.clone(),
                min_value: p.min_value,
                max_value: p.max_value,
                default_value: p.default_value,
                current_value: p.current_value,
            })
            .collect();

        let plugin_element = match &plugin.custom {
            PluginCustomState::Drums(state) => {
                resonance_drums::ui::view(state, &ui_params)
            }
            PluginCustomState::Amp(state) => {
                resonance_amp::ui::view(state, &ui_params)
            }
            PluginCustomState::Ir(state) => {
                resonance_ir::ui::view(state, &ui_params)
            }
            PluginCustomState::Generic => {
                resonance_plugin::ui::view_generic_params(&ui_params)
            }
        };

        let inst_id = selected_id;
        let mapped = plugin_element.map(move |event| {
            use resonance_plugin::ui::PluginUiEvent;
            match event {
                PluginUiEvent::SetParam(param_id, value) => Message::SetPluginParam(inst_id, param_id, value),
                PluginUiEvent::SelectPad(idx) => Message::DrumPadSelect(inst_id, idx),
                PluginUiEvent::BrowseFile => Message::PluginBrowseFile(inst_id),
                PluginUiEvent::PrevFile => Message::PluginPrevFile(inst_id),
                PluginUiEvent::NextFile => Message::PluginNextFile(inst_id),
            }
        });

        // Header with plugin name, optional Open Editor button, and close.
        let mut header = row![
            text(plugin.plugin_name.clone()).size(12).color(theme::ACCENT),
            Space::with_width(Length::Fill),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center);

        if plugin.has_gui {
            let label = if plugin.editor_open {
                "Close Editor"
            } else {
                "Open Editor"
            };
            let msg = if plugin.editor_open {
                Message::ClosePluginEditor(selected_id)
            } else {
                Message::OpenPluginEditor(selected_id)
            };
            header = header.push(
                button(text(label).size(9).color(theme::TEXT))
                    .on_press(msg)
                    .style(|_theme, status| theme::small_button_style(status))
                    .padding([2, 8]),
            );
        }

        header = header.push(
            button(text("\u{00d7}").size(14).color(theme::TEXT_DIM))
                .on_press(Message::TogglePluginPanel(selected_id))
                .style(|_theme, status| theme::small_button_style(status))
                .padding(2),
        );

        let panel_content = column![header, mapped]
            .spacing(6)
            .padding(10);

        let panel = container(
            scrollable(panel_content)
                .direction(scrollable::Direction::Vertical(scrollable::Scrollbar::default()))
        )
        .width(Length::Fill)
        .height(200)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::PANEL)),
            ..Default::default()
        });

        Some(panel.into())
    }
}
