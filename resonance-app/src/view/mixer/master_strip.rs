//! Master channel strip rendering. No instrument, no input/arm, no
//! per-channel routing — just the FX chain, the master fader, and the
//! Bounce-to-WAV button.

use iced::widget::{button, column, container, pick_list, text};
use iced::{Element, Length};
use resonance_audio::types::*;

use crate::message::*;
use crate::theme;
use crate::view::controls::{fader_section, fx_bypass_button};

use super::picks::PluginOwner;

impl crate::Resonance {
    pub(super) fn view_master_strip(
        &self,
        available_plugins: &[ScannedPlugin],
    ) -> Element<'_, Message> {
        let label = container(text("Master").size(14).color(theme::ACCENT))
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding([6, 4]);

        // FX bypass button, centered in its own row so the master strip
        // has a dedicated control spot (tracks and busses share a row
        // with other toggles; the master strip only has this one).
        let button_row = container(fx_bypass_button(
            self.master_fx_bypassed,
            Message::Master(MasterMessage::ToggleMasterFxBypass),
            10,
        ))
        .width(Length::Fill)
        .center_x(Length::Fill);

        // Plugin chain — every plugin is an effect.
        let mut plugin_section = column![].spacing(2).width(Length::Fill);
        for plugin in &self.master_plugins {
            plugin_section =
                plugin_section.push(self.view_plugin_slot_row(PluginOwner::Master, plugin, false));
        }

        // `+ FX` picker (filtered to effects). Only rendered when we
        // have at least one non-instrument plugin available.
        let fx_picker_element: Option<Element<'_, Message>> = if available_plugins.is_empty() {
            None
        } else {
            let effects: Vec<ScannedPlugin> = available_plugins
                .iter()
                .filter(|p| !p.is_instrument)
                .cloned()
                .collect();
            if effects.is_empty() {
                None
            } else {
                Some(
                    pick_list(effects, None::<ScannedPlugin>, |plugin: ScannedPlugin| {
                        Message::Master(MasterMessage::AddPluginToMaster(plugin))
                    })
                    .placeholder("+ FX")
                    .text_size(10)
                    .width(Length::Fill)
                    .into(),
                )
            }
        };

        let plugin_fill = container(plugin_section)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(iced::alignment::Vertical::Top);

        let fx_block = {
            let mut col = iced::widget::Column::new().spacing(0).width(Length::Fill);
            if let Some(fx) = fx_picker_element {
                col = col.push(fx);
            }
            col
        };

        let fader_block = fader_section(
            self.master_level_l,
            self.master_level_r,
            self.master_volume,
            |v| Message::Track(TrackMessage::SetMasterVolume(v)),
        );

        let bounce_btn: Element<'_, Message> = if self.io.bouncing {
            text("Bouncing...").size(8).color(theme::ACCENT).into()
        } else {
            button(text("Bounce").size(8).color(theme::TEXT))
                .on_press(Message::ProjectIo(ProjectIoMessage::BounceToWav))
                .style(|_theme, status| theme::small_button_style(status))
                .padding([2, 8])
                .into()
        };

        let bounce_row = container(bounce_btn)
            .width(Length::Fill)
            .center_x(Length::Fill);

        let strip_content = column![
            label,
            button_row,
            plugin_fill,
            fx_block,
            fader_block,
            bounce_row,
        ]
        .spacing(4)
        .padding(8)
        .width(theme::MASTER_STRIP_WIDTH);

        container(strip_content)
            .height(Length::Fill)
            .style(theme::panel_dark_outlined)
            .into()
    }
}
