/// Plugin panel views for the Resonance application.
use crate::message::Message;
use crate::state::*;
use crate::theme;
use iced::widget::{button, column, container, row, slider, text};
use iced::{alignment, Element, Length};

impl crate::Resonance {
    pub(crate) fn view_drums_panel<'a>(&self, plugin: &PluginSlotState, selected_pad: usize) -> Element<'a, Message> {
        let pid = plugin.instance_id;
        let pad_names = [
            "Kick", "Snare", "HH Close", "HH Open",
            "Tom Hi", "Tom Mid", "Tom Low", "Crash",
            "Ride", "Rimshot", "Clap", "Cowbell",
        ];

        // 4x3 pad grid
        let mut grid = column![].spacing(2);
        for row_idx in 0..3 {
            let mut grid_row = row![].spacing(2);
            for col_idx in 0..4 {
                let pad_idx = row_idx * 4 + col_idx;
                let is_selected = pad_idx == selected_pad;
                let name = pad_names[pad_idx];
                let bg = if is_selected {
                    iced::Color::from_rgb(0.25, 0.3, 0.45)
                } else {
                    iced::Color::from_rgb(0.2, 0.2, 0.24)
                };
                let border_color = if is_selected {
                    iced::Color::from_rgb(0.4, 0.5, 0.8)
                } else {
                    iced::Color::from_rgb(0.3, 0.3, 0.35)
                };
                let pad_btn = button(
                    container(text(name).size(7).color(theme::TEXT))
                        .center_x(Length::Fill)
                        .center_y(Length::Fill),
                )
                .on_press(Message::DrumPadSelect(pid, pad_idx))
                .width(Length::Fill)
                .height(28)
                .style(move |_theme, _status| iced::widget::button::Style {
                    background: Some(iced::Background::Color(bg)),
                    text_color: theme::TEXT,
                    border: iced::Border {
                        color: border_color,
                        width: if is_selected { 1.5 } else { 0.5 },
                        radius: 3.0.into(),
                    },
                    ..Default::default()
                });
                grid_row = grid_row.push(pad_btn);
            }
            grid = grid.push(grid_row);
        }

        // Per-pad controls: find volume/pan params for selected pad
        let pad_prefix = format!("Pad {} > ", selected_pad + 1);
        let mut pad_controls = column![
            text(format!("Pad: {}", pad_names[selected_pad])).size(8).color(theme::TEXT)
        ].spacing(1);

        for param in &plugin.params {
            if param.name.starts_with(&pad_prefix) || param.name == "Master Volume" {
                let param_id = param.id;
                let inst_id = pid;
                let label = if param.name == "Master Volume" {
                    "Master".to_string()
                } else {
                    param.name.strip_prefix(&pad_prefix).unwrap_or(&param.name).to_string()
                };
                let range = param.min_value..=param.max_value;
                let param_slider = slider(range, param.current_value, move |v| {
                    Message::SetPluginParam(inst_id, param_id, v)
                })
                .width(Length::Fill)
                .step(0.001);
                pad_controls = pad_controls.push(
                    column![
                        text(label).size(7).color(theme::TEXT_DIM),
                        param_slider,
                    ].spacing(0)
                );
            }
        }

        column![grid, pad_controls].spacing(4).into()
    }

    /// Unified file browser panel for plugins that load files (amp models, IR files, etc.).
    ///
    /// `display_name` is the current file name to display (or empty for "no file loaded").
    /// `info_text` is an optional extra info line (e.g. IR details).
    /// `file_count` / `current_index` track position in the file list.
    /// `param_names` controls which plugin parameters are shown as sliders.
    pub(crate) fn view_file_browser_panel<'a>(
        &self,
        plugin: &PluginSlotState,
        display_name: &str,
        info_text: Option<&str>,
        file_count: usize,
        current_index: usize,
        param_names: &[&str],
    ) -> Element<'a, Message> {
        let pid = plugin.instance_id;

        let display = if display_name.is_empty() {
            "No file loaded".to_string()
        } else {
            display_name.to_string()
        };
        let name_text = text(display).size(8).color(theme::TEXT);

        let count_text = if file_count > 0 {
            text(format!("{}/{}", current_index + 1, file_count)).size(7).color(theme::TEXT_DIM)
        } else {
            text("").size(7)
        };

        let prev_btn = button(text("<").size(8).color(theme::TEXT))
            .on_press(Message::PluginPrevFile(pid))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(1)
            .width(20);

        let browse_btn = button(text("Browse").size(7).color(theme::TEXT))
            .on_press(Message::PluginBrowseFile(pid))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(1);

        let next_btn = button(text(">").size(8).color(theme::TEXT))
            .on_press(Message::PluginNextFile(pid))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(1)
            .width(20);

        let nav_row = row![prev_btn, browse_btn, next_btn]
            .spacing(2)
            .align_y(alignment::Vertical::Center);

        let mut controls = column![name_text].spacing(2);

        // Add optional info text line
        if let Some(info) = info_text {
            controls = controls.push(text(info.to_string()).size(7).color(theme::TEXT_DIM));
        }

        controls = controls.push(count_text);
        controls = controls.push(nav_row);

        // Parameter sliders
        for param in &plugin.params {
            if param_names.iter().any(|n| *n == param.name) {
                let param_id = param.id;
                let inst_id = pid;
                let range = param.min_value..=param.max_value;
                let param_slider = slider(range, param.current_value, move |v| {
                    Message::SetPluginParam(inst_id, param_id, v)
                })
                .width(Length::Fill)
                .step(0.001);
                controls = controls.push(
                    column![
                        text(param.name.clone()).size(7).color(theme::TEXT_DIM),
                        param_slider,
                    ].spacing(0)
                );
            }
        }

        controls.into()
    }
}
