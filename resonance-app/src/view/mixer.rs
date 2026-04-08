/// Mixer view rendering for the Resonance application.
use crate::message::Message;
use crate::state::*;
use crate::theme;
use crate::util::{format_db, format_pan};
use iced::widget::{
    button, column, container, pick_list, row, scrollable, slider, text, Space,
};
use iced::{alignment, Color, Element, Font, Length};
use resonance_audio::types::*;

/// Convert linear amplitude to VU meter bar width (logarithmic/dB scale).
fn level_to_bar_width(level: f32, max_width: f32) -> f32 {
    if level < 0.0001 {
        return 0.0;
    }
    let db = 20.0 * level.log10();
    let normalized = (db + 60.0) / 66.0; // -60dB=0, +6dB=1
    normalized.clamp(0.0, 1.0) * max_width
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

/// Render a single horizontal VU meter bar.
fn meter_bar<'a>(level: f32, max_width: f32) -> Element<'a, Message> {
    let bar_width = level_to_bar_width(level, max_width);
    let color = level_color(level);

    let bar = container(Space::new(0.0, 0.0))
        .width(bar_width)
        .height(3)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(color)),
            ..Default::default()
        });

    container(bar)
        .width(Length::Fill)
        .height(3)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::METER_BG)),
            ..Default::default()
        })
        .into()
}

/// Render a stereo VU meter (L + R bars).
fn view_meter<'a>(level_l: f32, level_r: f32, strip_width: u16) -> Element<'a, Message> {
    let max_width = (strip_width as f32 - 12.0).max(0.0);
    column![meter_bar(level_l, max_width), meter_bar(level_r, max_width)]
        .spacing(1)
        .into()
}

impl crate::Resonance {
    pub(crate) fn view_mixer(&self) -> Element<'_, Message> {
        let mut sorted_tracks: Vec<&TrackState> = self.tracks.iter().collect();
        sorted_tracks.sort_by_key(|t| t.order);

        let mut strip_row = row![].spacing(2);
        for track in &sorted_tracks {
            strip_row = strip_row.push(self.view_channel_strip(track));
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

        let mixer_content = row![scrollable_strips, separator, master_strip]
            .height(Length::Fill);

        container(mixer_content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::BG)),
                ..Default::default()
            })
            .into()
    }

    fn view_channel_strip(&self, track: &TrackState) -> Element<'_, Message> {
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

        // Plugin chain with expandable params
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
            let expanded = plugin.expanded;

            let name_color = if expanded { theme::TEXT } else { theme::ACCENT };
            let name_btn = button(text(pname).size(9).color(name_color))
                .on_press(Message::TogglePluginPanel(pid))
                .style(|_theme, status| theme::small_button_style(status))
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

            if expanded {
                match &plugin.custom {
                    PluginCustomState::Drums { selected_pad } => {
                        plugin_section = plugin_section.push(self.view_drums_panel(plugin, *selected_pad));
                    }
                    PluginCustomState::Amp { model_name, file_list, current_index } => {
                        plugin_section = plugin_section.push(
                            self.view_file_browser_panel(plugin, model_name, None, file_list.len(), *current_index, &["Input Gain", "Output Gain"]),
                        );
                    }
                    PluginCustomState::Ir { ir_name, ir_info, file_list, current_index } => {
                        plugin_section = plugin_section.push(
                            self.view_file_browser_panel(plugin, ir_name, Some(ir_info.as_str()), file_list.len(), *current_index, &["Dry/Wet", "Output Gain"]),
                        );
                    }
                    PluginCustomState::Generic => {
                        for param in &plugin.params {
                            let param_id = param.id;
                            let inst_id = pid;
                            let range = param.min_value..=param.max_value;
                            let param_slider = slider(
                                range,
                                param.current_value,
                                move |v| Message::SetPluginParam(inst_id, param_id, v),
                            )
                            .width(Length::Fill)
                            .step(0.001);

                            let param_label = text(param.name.clone()).size(8).color(theme::TEXT_DIM);
                            let param_value_text = text(format!("{:.2}", param.current_value))
                                .size(8)
                                .font(Font::MONOSPACE)
                                .color(theme::TEXT_DIM);

                            let param_row = column![
                                row![param_label, Space::with_width(Length::Fill), param_value_text]
                                    .spacing(2),
                                param_slider,
                            ]
                            .spacing(1);
                            plugin_section = plugin_section.push(param_row);
                        }
                    }
                }
            }
        }

        // FX picker
        if !self.available_plugins.is_empty() {
            let track_id = track.id;
            let fx_picker = pick_list(
                self.available_plugins.clone(),
                None::<ScannedPlugin>,
                move |plugin: ScannedPlugin| Message::AddPluginToTrack(track_id, plugin),
            )
            .placeholder("+ FX")
            .text_size(10)
            .width(Length::Fill);
            plugin_section = plugin_section.push(fx_picker);
        }

        // Pan control
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

        // Volume fader (horizontal for now)
        let vol_slider = slider(-60.0..=6.0f32, track.volume, {
            let id = track.id;
            move |v| Message::SetTrackVolume(id, v)
        })
        .width(Length::Fill)
        .step(0.1);

        let vol_label = format_db(track.volume);
        let vol_row = row![
            text("Vol").size(9).color(theme::TEXT_DIM),
            Space::with_width(4),
            vol_slider,
            Space::with_width(4),
            text(vol_label).size(9).font(Font::MONOSPACE).color(theme::TEXT_DIM),
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center);

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

        let meter_section = view_meter(track.level_l, track.level_r, theme::MIXER_STRIP_WIDTH);

        let strip_content = column![
            track_name,
            button_row,
            plugin_section,
            pan_row,
            vol_row,
            meter_section,
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

        let vol_slider = slider(-60.0..=6.0f32, self.master_volume, Message::SetMasterVolume)
            .width(Length::Fill)
            .step(0.1);

        let vol_label = format_db(self.master_volume);

        let vol_row = row![
            text("Vol").size(9).color(theme::TEXT_DIM),
            Space::with_width(4),
            vol_slider,
            Space::with_width(4),
            text(vol_label).size(9).font(Font::MONOSPACE).color(theme::TEXT_DIM),
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center);

        let meter_section = view_meter(self.master_level_l, self.master_level_r, theme::MASTER_STRIP_WIDTH);

        let strip_content = column![
            label,
            Space::with_height(Length::Fill),
            vol_row,
            meter_section,
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
}
