//! The host-drawn generic plugin window (mixer-cleanup.md §4).
//!
//! "Open" always opens a window. A plugin with its own GUI opens its
//! floating editor; one without — or one that is missing, or whose editor
//! refused to open — gets this: a draggable in-app panel layered over the
//! app (between the base view and the modal overlays, like the palette and
//! preset-browser cards), with the plugin's name and owner in its title
//! bar, the preset bar under it and the generic parameter list below. A
//! missing plugin shows its recovery body instead of parameters.
//!
//! The body and the preset bar are the former bottom-panel ones
//! (`view/mixer/plugin_panel.rs`), moved, not rewritten. State lives in
//! [`crate::state::PluginWindowState`]; routing and dragging in
//! `update/plugin_window.rs`.

use iced::widget::{
    button, column, container, mouse_area, opaque, pin, row, scrollable, text, Space,
};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::state::*;
use crate::theme;

/// Outer width of the window.
pub const PLUGIN_WINDOW_WIDTH: f32 = 460.0;
/// Most the parameter body grows before it scrolls.
const PLUGIN_WINDOW_BODY_MAX_HEIGHT: f32 = 380.0;

impl crate::Resonance {
    /// The generic plugin window, placed at its position over the whole
    /// window, or `None` when none is open. Everything outside the card is
    /// event-transparent, so the app under it stays live.
    pub(crate) fn view_plugin_window(&self) -> Option<Element<'_, Message>> {
        if !crate::update::plugin_window::visible(self) {
            return None;
        }
        let window = self.ui.mixer.plugin_window?;
        let selected_id = window.instance_id;
        let plugin = self.plugin_slot(selected_id)?;

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
                            crate::plugin_ui::view_generic_params(&ui_params(plugin))
                        }
                    };
                    plugin_element.map(move |event| {
                        use crate::plugin_ui::PluginUiEvent;
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

        // Title bar: plugin name, owner, the editor toggle for a plugin
        // that has its own window (the fallback when it refused to open),
        // and close. Pressing anywhere else on it starts a drag.
        let mut title = row![
            text(plugin.plugin_name.clone())
                .size(12)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::ACCENT),
            text(self.plugin_owner_name(selected_id))
                .size(11)
                .color(theme::TEXT_3),
            Space::new().width(Length::Fill),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center);

        if plugin.has_gui && plugin.availability.reason().is_none() {
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
            title = title.push(
                button(text(label).size(9).color(theme::TEXT))
                    .on_press(msg)
                    .style(|_theme, status| theme::small_button_style(status))
                    .padding([2, 8]),
            );
        }

        title = title.push(
            button(text("\u{00d7}").size(14).color(theme::TEXT_DIM))
                .on_press(Message::Plugin(PluginMessage::ClosePluginWindow(
                    selected_id,
                )))
                .style(|_theme, status| theme::small_button_style(status))
                .padding(2),
        );

        let title_bar = mouse_area(
            container(title)
                .width(Length::Fill)
                .padding([6, 10])
                .style(|_theme| container::Style {
                    background: Some(iced::Background::Color(theme::BG_3)),
                    border: iced::Border {
                        radius: iced::border::Radius::new(0.0)
                            .top_left(theme::RADIUS_LG)
                            .top_right(theme::RADIUS_LG),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        )
        .on_press(Message::Plugin(PluginMessage::PluginWindowDrag(
            PluginWindowDrag::Begin,
        )))
        .interaction(iced::mouse::Interaction::Grab);

        // The preset bar: any plugin with an instance behind it (§6.6).
        let mut content = column![].spacing(8).padding(10).width(Length::Fill);
        if plugin.availability.reason().is_none() {
            content = content.push(crate::view::preset_browser::preset_bar(self, plugin));
        }
        content = content.push(mapped);

        let body = container(
            scrollable(content).direction(scrollable::Direction::Vertical(
                scrollable::Scrollbar::default(),
            )),
        )
        .width(Length::Fill)
        .max_height(PLUGIN_WINDOW_BODY_MAX_HEIGHT);

        let card = container(column![title_bar, body].spacing(0))
            .width(PLUGIN_WINDOW_WIDTH)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::BG_2)),
                border: iced::Border {
                    color: theme::LINE,
                    width: 1.0,
                    radius: theme::RADIUS_LG.into(),
                },
                shadow: iced::Shadow {
                    color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.45),
                    offset: iced::Vector::new(0.0, 6.0),
                    blur_radius: 18.0,
                },
                ..Default::default()
            });

        Some(
            pin(opaque(card))
                .x(window.position.x)
                .y(window.position.y)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        )
    }

    /// The window title's owner label: the track or bus the plugin sits
    /// on, or "Master".
    fn plugin_owner_name(&self, instance_id: resonance_audio::types::PluginInstanceId) -> String {
        let has =
            |plugins: &[PluginSlotState]| plugins.iter().any(|p| p.instance_id == instance_id);
        if let Some(t) = self.registry.tracks.iter().find(|t| has(&t.plugins)) {
            return t.name.clone();
        }
        if let Some(b) = self.registry.busses.iter().find(|b| has(&b.plugins)) {
            return b.name.clone();
        }
        "Master".to_string()
    }

    /// The window body for a slot the engine could not fill: what is
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
            self.ui.view_caches.instrument_plugins.clone()
        } else {
            self.ui.view_caches.fx_plugins.clone()
        }
    }
}

/// The generic parameter panel's rows. `hidden` params are dropped rather
/// than drawn: CLAP's IS_HIDDEN is the plugin asking that a parameter not
/// be presented as a control (it stays in the app's mirror because it is
/// still automatable and still saved — ba todo #1290).
fn ui_params(plugin: &PluginSlotState) -> Vec<crate::plugin_ui::UiParam> {
    plugin
        .params
        .iter()
        .filter(|p| !p.hidden)
        .map(|p| crate::plugin_ui::UiParam {
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
