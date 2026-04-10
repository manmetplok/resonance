/// Plugin UI types and shared widgets for Resonance plugins.
///
/// This module provides the shared event type, param data struct, theme constants,
/// and reusable widget helpers that plugin UIs are built from.

pub use iced;

use iced::widget::{button, column, row, slider, text};
use iced::{alignment, Element, Font, Length};

// -- Data types ---------------------------------------------------------------

/// Parameter data passed from the host to plugin views.
#[derive(Debug, Clone)]
pub struct UiParam {
    pub id: u32,
    pub name: String,
    pub min_value: f64,
    pub max_value: f64,
    pub default_value: f64,
    pub current_value: f64,
}

/// Events emitted by plugin UIs, mapped to host messages by the app.
#[derive(Debug, Clone)]
pub enum PluginUiEvent {
    SetParam(u32, f64),
    BrowseFile,
    PrevFile,
    NextFile,
}

// -- Theme constants ----------------------------------------------------------

pub const TEXT: iced::Color = iced::Color::from_rgb(
    0xe0 as f32 / 255.0,
    0xe0 as f32 / 255.0,
    0xe0 as f32 / 255.0,
);

pub const TEXT_DIM: iced::Color = iced::Color::from_rgb(
    0x80 as f32 / 255.0,
    0x80 as f32 / 255.0,
    0x80 as f32 / 255.0,
);

pub fn small_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => iced::Color::from_rgb(0.22, 0.22, 0.22),
        button::Status::Pressed => iced::Color::from_rgb(0.15, 0.15, 0.15),
        _ => iced::Color::TRANSPARENT,
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: TEXT,
        border: iced::Border {
            color: iced::Color::TRANSPARENT,
            width: 0.0,
            radius: 2.0.into(),
        },
        ..Default::default()
    }
}

// -- Shared widgets -----------------------------------------------------------

/// Render a generic param slider view for any plugin.
pub fn view_generic_params<'a>(params: &[UiParam]) -> Element<'a, PluginUiEvent> {
    let mut controls = column![].spacing(2);
    for param in params {
        let param_id = param.id;
        let range = param.min_value..=param.max_value;
        let param_slider = slider(range, param.current_value, move |v| {
            PluginUiEvent::SetParam(param_id, v)
        })
        .width(Length::Fill)
        .step(0.001);

        let param_label = text(param.name.clone()).size(8).color(TEXT_DIM);
        let param_value_text = text(format!("{:.2}", param.current_value))
            .size(8)
            .font(Font::MONOSPACE)
            .color(TEXT_DIM);

        let param_row = column![
            row![param_label, iced::widget::Space::with_width(Length::Fill), param_value_text]
                .spacing(2),
            param_slider,
        ]
        .spacing(1);
        controls = controls.push(param_row);
    }
    controls.into()
}

/// Render a file browser panel with prev/browse/next navigation and param sliders.
pub fn view_file_browser(
    display_name: &str,
    info_text: Option<&str>,
    file_count: usize,
    current_index: usize,
    params: &[UiParam],
    shown_param_names: &[&str],
) -> Element<'static, PluginUiEvent> {
    let display = if display_name.is_empty() {
        "No file loaded".to_string()
    } else {
        display_name.to_string()
    };
    let name_text = text(display).size(8).color(TEXT);

    let count_text = if file_count > 0 {
        text(format!("{}/{}", current_index + 1, file_count))
            .size(7)
            .color(TEXT_DIM)
    } else {
        text("").size(7)
    };

    let prev_btn = button(text("<").size(8).color(TEXT))
        .on_press(PluginUiEvent::PrevFile)
        .style(|_theme, status| small_button_style(status))
        .padding(1)
        .width(20);

    let browse_btn = button(text("Browse").size(7).color(TEXT))
        .on_press(PluginUiEvent::BrowseFile)
        .style(|_theme, status| small_button_style(status))
        .padding(1);

    let next_btn = button(text(">").size(8).color(TEXT))
        .on_press(PluginUiEvent::NextFile)
        .style(|_theme, status| small_button_style(status))
        .padding(1)
        .width(20);

    let nav_row = row![prev_btn, browse_btn, next_btn]
        .spacing(2)
        .align_y(alignment::Vertical::Center);

    let mut controls = column![name_text].spacing(2);

    if let Some(info) = info_text {
        controls = controls.push(text(info.to_string()).size(7).color(TEXT_DIM));
    }

    controls = controls.push(count_text);
    controls = controls.push(nav_row);

    // Parameter sliders for the specified param names
    for param in params {
        if shown_param_names.iter().any(|n| *n == param.name) {
            let param_id = param.id;
            let range = param.min_value..=param.max_value;
            let param_slider = slider(range, param.current_value, move |v| {
                PluginUiEvent::SetParam(param_id, v)
            })
            .width(Length::Fill)
            .step(0.001);
            controls = controls.push(
                column![
                    text(param.name.clone()).size(7).color(TEXT_DIM),
                    param_slider,
                ]
                .spacing(0),
            );
        }
    }

    controls.into()
}
