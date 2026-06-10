//! Bus channel strip rendering. Trimmer than a track strip: no
//! mono/monitor/arm, no instrument slot, no input device picker, no
//! output selector (busses always go to master).

use iced::widget::{column, container, pick_list, row, scrollable, text, Space};
use iced::{alignment, Element, Font, Length};
use resonance_audio::types::*;

use crate::message::*;
use crate::state::*;
use crate::theme;
use crate::util::format_pan;
use crate::view::controls::{bus_remove_button, fader_section, fx_bypass_button, mute_button};
use crate::view::knob::pan_knob;

use super::picks::PluginOwner;

impl crate::Resonance {
    pub(super) fn view_bus_strip(
        &self,
        bus: &BusState,
        available_plugins: &[ScannedPlugin],
    ) -> Element<'_, Message> {
        // Bus strips show their name in the warm/amber accent — matches
        // the design's "audio-or-aggregate" semantic split.
        let bus_name = container(text(bus.name.clone()).size(13).color(theme::WARM))
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding([6, 4]);

        // Mute + FX bypass + Remove buttons — same icons as the track header.
        // Fixed-height container so the row doesn't collapse inside a
        // Length::Fill strip column (same quirk as the track strip).
        let bus_id = bus.id;
        let inner_row = row![
            mute_button(
                bus.muted,
                Message::Bus(BusMessage::ToggleBusMute(bus_id)),
                12
            ),
            fx_bypass_button(
                bus.fx_bypassed,
                Message::Bus(BusMessage::ToggleBusFxBypass(bus_id)),
                12,
            ),
            Space::new().width(Length::Fill),
            bus_remove_button(bus_id, 12),
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center);
        let button_row = container(inner_row)
            .width(Length::Fill)
            .height(Length::Fixed(24.0))
            .padding([2, 4]);

        // Plugin chain (all effects — no instrument slot on busses).
        let mut plugin_section = column![].spacing(4).width(Length::Fill);
        for plugin in &bus.plugins {
            plugin_section = plugin_section.push(self.view_plugin_slot_row(
                PluginOwner::Bus(bus_id),
                plugin,
                false,
            ));
        }

        // Extract the +FX picker so it can dock above the pan knob (same
        // treatment as track strips). Options come from the cached FX
        // filter on `view_caches` — cloning that Rc is a refcount bump.
        let _ = available_plugins;
        let fx_picker_element: Option<Element<'_, Message>> =
            if self.view_caches.fx_plugins.is_empty() {
                None
            } else {
                Some(
                    pick_list(
                        self.view_caches.fx_plugins.clone(),
                        None::<ScannedPlugin>,
                        move |plugin: ScannedPlugin| {
                            Message::Bus(BusMessage::AddPluginToBus(bus_id, plugin))
                        },
                    )
                    .placeholder("+ FX")
                    .text_size(10)
                    .width(Length::Fill)
                    .into(),
                )
            };

        // Pan knob — vertical drag to change, double-click to reset.
        let pan_ctrl = pan_knob(bus.pan, move |v| {
            Message::Bus(BusMessage::SetBusPan(bus_id, v))
        });
        let pan_label = format_pan(bus.pan);
        let pan_row = row![
            text("Pan").size(9).color(theme::TEXT_DIM),
            Space::new().width(Length::Fill),
            pan_ctrl,
            Space::new().width(Length::Fill),
            text(pan_label)
                .size(9)
                .font(Font::MONOSPACE)
                .color(theme::TEXT_DIM),
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center);

        let fx_pan_block = {
            let mut col = iced::widget::Column::new().spacing(0).width(Length::Fill);
            if let Some(fx) = fx_picker_element {
                col = col.push(fx);
            }
            col.push(pan_row)
        };

        let fader_block = fader_section(bus.level_l, bus.level_r, bus.volume, move |v| {
            Message::Bus(BusMessage::SetBusVolume(bus_id, v))
        });

        // FX list scrolls inside its own area between the buttons and
        // the pan/fader block so adding plugins never pushes the fader
        // off the strip.
        let plugin_fill = iced::widget::Scrollable::with_direction(
            plugin_section,
            scrollable::Direction::Vertical(
                scrollable::Scrollbar::default().width(4).scroller_width(4),
            ),
        )
        .width(Length::Fill)
        .height(Length::Fill);

        let strip_content = column![bus_name, button_row, plugin_fill, fx_pan_block, fader_block,]
            .spacing(6)
            .padding([12, 10])
            .width(theme::MIXER_STRIP_WIDTH)
            .height(Length::Fill);

        container(strip_content)
            .height(Length::Fixed(theme::BUS_STRIP_HEIGHT as f32))
            .style(theme::card_warm)
            .into()
    }
}
