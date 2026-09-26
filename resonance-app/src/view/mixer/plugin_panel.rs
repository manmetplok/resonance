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

        let inst_id = selected_id;
        // A slot with nothing behind it has no parameters to draw — and
        // drawing an empty generic panel is exactly the "dead slot that
        // looks like a working plugin" this surface exists to end. The
        // body becomes the recovery surface instead (ba doc #275 P5,
        // todo #1309).
        let mapped: Element<'_, Message> = match plugin.availability.reason() {
            Some(reason) => self.missing_plugin_body(plugin, reason),
            None => {
                // The parameter list sits in a `lazy` region keyed on every
                // field it draws, so the Mixer's fast meter tick reuses the
                // built widgets instead of cloning each parameter's name and
                // text and rebuilding a row per parameter every frame
                // (ui-work.md §11, review VIEW-26).
                let fp = plugin_params_fingerprint(plugin);
                iced::widget::lazy(fp, move |_: &u64| -> Element<'static, Message> {
                    let plugin_element = match &plugin.custom {
                        PluginCustomState::Generic => {
                            resonance_plugin::ui::view_generic_params(&ui_params(plugin))
                        }
                    };
                    plugin_element.map(move |event| {
                        use resonance_plugin::ui::PluginUiEvent;
                        match event {
                            PluginUiEvent::SetParam(param_id, value) => Message::Plugin(
                                PluginMessage::SetPluginParam(inst_id, param_id, value),
                            ),
                        }
                    })
                })
                .into()
            }
        };

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

    /// The panel body for a slot the engine could not fill: what is
    /// missing, why, and the two ways out of it.
    ///
    /// This is the GUI half of the replace capability, and it lives here
    /// rather than on the strip for two reasons. The strip's slot row is
    /// a 140 px budget already carrying four icon controls, and — more
    /// to the point — this panel is the ONE plugin surface that serves
    /// all three chains, so a missing plugin on a bus or on the master
    /// gets the same recovery affordance a track plugin does. The strip
    /// pill's warning tint is what leads the user here.
    ///
    /// The picker is the whole gesture: choosing the SAME plugin (which
    /// the catalog only offers once a rescan has found it again)
    /// relocates the slot and brings its saved settings back; choosing a
    /// different one swaps it and keeps the chain position. Removal is
    /// deliberately not repeated here — the strip's × already does it,
    /// and the consequence is stated rather than made one click easier.
    fn missing_plugin_body(&self, plugin: &PluginSlotState, reason: &str) -> Element<'_, Message> {
        let instance_id = plugin.instance_id;
        let candidates = self.replacement_candidates(instance_id);

        let mut body = column![
            text(format!(
                "\u{26a0} {} is not available on this machine",
                plugin.plugin_name
            ))
            .size(12)
            .color(theme::BAD),
            text(reason.to_owned()).size(10).color(theme::TEXT_2),
            text(format!(
                "{}  \u{2014}  {}",
                plugin.clap_plugin_id, plugin.clap_file_path
            ))
            .size(9)
            .color(theme::TEXT_3),
            text(
                "Its settings are kept with this slot: reinstall the plugin and rescan, \
                 or pick a replacement below. Removing the slot discards them."
            )
            .size(10)
            .color(theme::TEXT_2),
        ]
        .spacing(6);

        if candidates.is_empty() {
            body = body.push(
                text("No plugins have been scanned yet.")
                    .size(10)
                    .color(theme::TEXT_3),
            );
        } else {
            body = body.push(
                iced::widget::pick_list(
                    candidates,
                    None::<resonance_audio::types::ScannedPlugin>,
                    move |plugin: resonance_audio::types::ScannedPlugin| {
                        Message::Plugin(PluginMessage::ReplacePlugin {
                            instance_id,
                            plugin,
                        })
                    },
                )
                .placeholder("Replace with\u{2026}")
                .text_size(12)
                .padding([8, 10])
                .width(Length::Fixed(320.0)),
            );
        }

        body.into()
    }

    /// Which plugins may take over a slot: instruments for a track's
    /// instrument slot, effects everywhere else.
    ///
    /// The distinction is not cosmetic — an instrument in an insert slot
    /// receives no MIDI and an effect in the instrument slot leaves the
    /// track with no sound source — and it is the same split the
    /// `+ Add instrument` / `+ Add to chain` pickers already make.
    fn replacement_candidates(
        &self,
        instance_id: resonance_audio::types::PluginInstanceId,
    ) -> std::rc::Rc<[resonance_audio::types::ScannedPlugin]> {
        let is_instrument_slot = self.registry.tracks.iter().any(|t| {
            crate::plugin_chain::instrument_slot(self, t)
                .and_then(|i| t.plugins.get(i))
                .is_some_and(|p| p.instance_id == instance_id)
        });
        if is_instrument_slot {
            self.view_caches.instrument_plugins.clone()
        } else {
            self.view_caches.fx_plugins.clone()
        }
    }
}

/// The generic parameter panel's rows. `hidden` params are dropped rather
/// than drawn: CLAP's IS_HIDDEN is the plugin asking that a parameter not
/// be presented as a control (it stays in the app's mirror because it is
/// still automatable and still saved — ba todo #1290).
fn ui_params(plugin: &PluginSlotState) -> Vec<resonance_plugin::ui::UiParam> {
    plugin
        .params
        .iter()
        .filter(|p| !p.hidden)
        .map(|p| resonance_plugin::ui::UiParam {
            id: p.id,
            name: p.name.clone(),
            min_value: p.min_value,
            max_value: p.max_value,
            default_value: p.default_value,
            current_value: p.current_value,
            // The plugin's own formatting, when it has one. Where
            // this panel printed "0.40" it now prints the "40 %"
            // the plugin's editor shows.
            text: p.text.clone(),
            stepped: p.stepped,
        })
        .collect()
}

/// Lazy key of the generic parameter panel: the instance plus every
/// field [`ui_params`] carries into the rows. Hashing is allocation-free,
/// so a frame with nothing changed costs one pass over the params and no
/// widget rebuild.
pub(crate) fn plugin_params_fingerprint(plugin: &PluginSlotState) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    plugin.instance_id.hash(&mut h);
    std::mem::discriminant(&plugin.custom).hash(&mut h);
    for p in plugin.params.iter().filter(|p| !p.hidden) {
        p.id.hash(&mut h);
        p.name.hash(&mut h);
        p.min_value.to_bits().hash(&mut h);
        p.max_value.to_bits().hash(&mut h);
        p.default_value.to_bits().hash(&mut h);
        p.current_value.to_bits().hash(&mut h);
        p.text.hash(&mut h);
        p.stepped.hash(&mut h);
    }
    h.finish()
}
