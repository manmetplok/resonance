/// Drums plugin UI: 4x3 pad grid with per-pad volume/pan/mute controls.

use resonance_plugin::ui::*;
use resonance_plugin::ui::iced::widget::{button, column, container, row, slider, text};
use resonance_plugin::ui::iced::{Element, Length};

#[derive(Debug, Clone)]
pub struct DrumsUiState {
    pub selected_pad: usize,
}

impl Default for DrumsUiState {
    fn default() -> Self {
        Self { selected_pad: 0 }
    }
}

const PAD_NAMES: [&str; 12] = [
    "Kick", "Snare", "HH Close", "HH Open",
    "Tom Hi", "Tom Mid", "Tom Low", "Crash",
    "Ride", "Rimshot", "Clap", "Cowbell",
];

pub fn view(state: &DrumsUiState, params: &[UiParam]) -> Element<'static, PluginUiEvent> {
    let selected_pad = state.selected_pad;

    // 4x3 pad grid
    let mut grid = column![].spacing(2);
    for row_idx in 0..3 {
        let mut grid_row = row![].spacing(2);
        for col_idx in 0..4 {
            let pad_idx = row_idx * 4 + col_idx;
            let is_selected = pad_idx == selected_pad;
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
                container(text(PAD_NAMES[pad_idx]).size(7).color(TEXT))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill),
            )
            .on_press(PluginUiEvent::SelectPad(pad_idx))
            .width(Length::Fill)
            .height(28)
            .style(move |_theme, _status| iced::widget::button::Style {
                background: Some(iced::Background::Color(bg)),
                text_color: TEXT,
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
    let pad_prefix = format!("Pad {} ", selected_pad);
    let mut pad_controls = column![
        text(format!("Pad: {}", PAD_NAMES[selected_pad])).size(8).color(TEXT)
    ]
    .spacing(1);

    for param in params {
        if param.name.starts_with(&pad_prefix) || param.name == "Master Volume" {
            let param_id = param.id;
            let label = if param.name == "Master Volume" {
                "Master".to_string()
            } else {
                param
                    .name
                    .strip_prefix(&pad_prefix)
                    .unwrap_or(&param.name)
                    .to_string()
            };
            let range = param.min_value..=param.max_value;
            let param_slider = slider(range, param.current_value, move |v| {
                PluginUiEvent::SetParam(param_id, v)
            })
            .width(Length::Fill)
            .step(0.001);
            pad_controls = pad_controls.push(
                column![text(label).size(7).color(TEXT_DIM), param_slider,].spacing(0),
            );
        }
    }

    column![grid, pad_controls].spacing(4).into()
}
