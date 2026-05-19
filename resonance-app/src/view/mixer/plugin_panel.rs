//! Bottom-panel plugin UI. When the user clicks a plugin slot in any
//! strip, the panel shows that plugin's params (or, for plugins with a
//! floating editor, an Open/Close Editor button instead of the params).

use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::state::*;
use crate::theme;

impl crate::Resonance {
    /// Bottom panel showing the selected plugin's UI.
    pub(super) fn view_plugin_panel(&self) -> Option<Element<'_, Message>> {
        let selected_id = self.mixer.selected_plugin?;

        // Find the plugin across all tracks, busses, and the master chain.
        let plugin = self
            .registry
            .tracks
            .iter()
            .flat_map(|t| t.plugins.iter())
            .chain(self.registry.busses.iter().flat_map(|b| b.plugins.iter()))
            .chain(self.master_plugins.iter())
            .find(|p| p.instance_id == selected_id)?;

        let ui_params: Vec<resonance_plugin::ui::UiParam> = plugin
            .params
            .iter()
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
            PluginCustomState::Generic => resonance_plugin::ui::view_generic_params(&ui_params),
        };

        let inst_id = selected_id;
        let mapped = plugin_element.map(move |event| {
            use resonance_plugin::ui::PluginUiEvent;
            match event {
                PluginUiEvent::SetParam(param_id, value) => {
                    Message::Plugin(PluginMessage::SetPluginParam(inst_id, param_id, value))
                }
            }
        });

        // Header with plugin name, optional Open Editor button, and close.
        let mut header = row![
            text(plugin.plugin_name.clone())
                .size(12)
                .color(theme::ACCENT),
            Space::new().width(Length::Fill),
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
                Message::Plugin(PluginMessage::ClosePluginEditor(selected_id))
            } else {
                Message::Plugin(PluginMessage::OpenPluginEditor(selected_id))
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
                .on_press(Message::Plugin(PluginMessage::TogglePluginPanel(
                    selected_id,
                )))
                .style(|_theme, status| theme::small_button_style(status))
                .padding(2),
        );

        let panel_content = column![header, mapped].spacing(6).padding(10);

        let panel = container(scrollable(panel_content).direction(
            scrollable::Direction::Vertical(scrollable::Scrollbar::default()),
        ))
        .width(Length::Fill)
        .height(200)
        .style(theme::panel_bg);

        Some(panel.into())
    }
}
