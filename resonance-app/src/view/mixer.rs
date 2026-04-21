/// Mixer view rendering for the Resonance application.
use crate::message::*;
use crate::state::*;
use crate::theme::{self, fa};
use crate::util::format_pan;
use crate::view::controls::{
    bus_remove_button, fader_section, fx_bypass_button, meter_v, monitor_button, mono_button,
    mute_button, record_arm_button, solo_button,
};
use crate::view::knob::pan_knob;
use iced::widget::{button, column, container, pick_list, row, scrollable, text, Space};
use iced::{alignment, Color, Element, Font, Length};
use resonance_audio::types::*;

/// Which container a plugin slot belongs to. Used so `view_plugin_slot_row`
/// can emit the right remove message regardless of whether it's rendering
/// a track's plugin or a bus's plugin.
#[derive(Debug, Clone, Copy)]
enum PluginOwner {
    Track(TrackId),
    Bus(BusId),
    Master,
}

/// Wrapper type for the output-destination pick_list so iced can render
/// it via `Display` and `PartialEq`. Variants correspond 1:1 with
/// `TrackOutput` but carry a display name for the chosen bus.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OutputChoice {
    label: String,
    output: TrackOutput,
}

impl std::fmt::Display for OutputChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// Wrapper so the input-port pick_list can render 1-based channel
/// numbers and stereo pair labels without reaching into track state.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PortChoice {
    /// 0-indexed channel number on the device.
    index: u16,
    /// True if the track is mono — the label shows "In N"; false shows
    /// "In N/N+1" so the user sees which pair the stereo track reads.
    mono: bool,
}

impl std::fmt::Display for PortChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let one_based = self.index + 1;
        if self.mono {
            write!(f, "In {}", one_based)
        } else {
            write!(f, "In {}/{}", one_based, one_based + 1)
        }
    }
}


impl crate::Resonance {
    pub(crate) fn view_mixer(&self) -> Element<'_, Message> {
        let sorted_tracks = self.sorted_tracks();
        let sorted_busses = self.sorted_busses();
        let available_plugins = self.available_plugins.clone();

        // -- Top row: track strips + master strip on the right. --
        // Skip sub-tracks whose parent is currently collapsed in the
        // mixer UI. The collapse state is app-side only (not persisted).
        let mut track_strip_row = row![].spacing(2);
        for track in &sorted_tracks {
            if let Some(link) = track.sub_track {
                if !self
                    .mixer
                    .expanded_sub_track_parents
                    .contains(&link.parent_track_id)
                {
                    continue;
                }
            }
            track_strip_row =
                track_strip_row.push(self.view_channel_strip(track, &available_plugins));
        }
        // Construct the scrollable with its horizontal direction up front.
        // `scrollable(content)` would default to Vertical and run its
        // `validate()` debug assertion before the chained `.direction(...)`
        // has a chance to change it — and `track_strip_row`'s size hint is
        // Fill-height now that each strip claims Length::Fill vertically.
        let scrollable_tracks = iced::widget::Scrollable::with_direction(
            track_strip_row,
            scrollable::Direction::Horizontal(scrollable::Scrollbar::default()),
        )
        .width(Length::Fill);
        let master_strip = self.view_master_strip(&available_plugins);
        let v_separator_tracks =
            container(Space::new(1, Length::Fill)).style(theme::separator_bg);
        let tracks_area = row![scrollable_tracks, v_separator_tracks, master_strip]
            .height(Length::FillPortion(1));

        // -- Bottom row: bus strips + "+ Bus" button on the right. --
        let mut bus_strip_row = row![].spacing(2);
        for bus in &sorted_busses {
            bus_strip_row = bus_strip_row.push(self.view_bus_strip(bus, &available_plugins));
        }
        let scrollable_busses = iced::widget::Scrollable::with_direction(
            bus_strip_row,
            scrollable::Direction::Horizontal(scrollable::Scrollbar::default()),
        )
        .width(Length::Fill);
        let add_bus_strip = self.view_add_bus_strip();
        let v_separator_busses =
            container(Space::new(1, Length::Fill)).style(theme::separator_bg);
        let busses_area = row![scrollable_busses, v_separator_busses, add_bus_strip]
            .height(Length::FillPortion(1));

        let h_sep_mid = container(Space::new(Length::Fill, 1)).style(theme::separator_bg);

        let mut mixer_col = column![].spacing(0);
        mixer_col = mixer_col.push(tracks_area);
        mixer_col = mixer_col.push(h_sep_mid);
        mixer_col = mixer_col.push(busses_area);

        if let Some(panel) = self.view_plugin_panel() {
            let h_sep = container(Space::new(Length::Fill, 1)).style(theme::separator_bg);
            mixer_col = mixer_col.push(h_sep);
            mixer_col = mixer_col.push(panel);
        }

        container(mixer_col)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::base_bg)
            .into()
    }

    /// Small "+ Bus" strip that lives in the same slot the master strip
    /// occupies in the top row, but in the bus row. Clicking it dispatches
    /// `Message::Bus(BusMessage::AddBus)`.
    fn view_add_bus_strip(&self) -> Element<'_, Message> {
        let label = container(
            text("Busses").size(11).color(theme::TEXT_DIM),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .padding([6, 4]);

        let add_btn = button(text("+ Bus").size(11).color(theme::TEXT))
            .on_press(Message::Bus(BusMessage::AddBus))
            .style(|_theme, status| theme::small_button_style(status))
            .padding([4, 10]);

        let content = column![
            label,
            container(add_btn)
                .width(Length::Fill)
                .center_x(Length::Fill),
            Space::with_height(Length::Fill),
        ]
        .spacing(4)
        .padding(8)
        .width(theme::MASTER_STRIP_WIDTH);

        container(content)
            .height(Length::Fill)
            .style(theme::panel_dark_outlined)
            .into()
    }

    /// Render a single plugin slot row (name button + remove button).
    /// If `is_instrument_slot` is true, the name is tinted to distinguish it.
    fn view_plugin_slot_row(
        &self,
        owner: PluginOwner,
        plugin: &PluginSlotState,
        is_instrument_slot: bool,
    ) -> Element<'_, Message> {
        let pname: String = if plugin.plugin_name.chars().count() > 14 {
            let mut s: String = plugin.plugin_name.chars().take(12).collect();
            s.push_str("..");
            s
        } else {
            plugin.plugin_name.clone()
        };
        let pid = plugin.instance_id;
        // Plugins that expose a floating editor (has_gui) are driven
        // entirely from that window — clicking the name in the strip
        // toggles the editor open/closed rather than showing the
        // generic params in the bottom panel. Plugins without a
        // floating editor still fall back to the bottom panel path.
        let (click_msg, is_selected) = if plugin.has_gui {
            let msg = if plugin.editor_open {
                Message::Plugin(PluginMessage::ClosePluginEditor(pid))
            } else {
                Message::Plugin(PluginMessage::OpenPluginEditor(pid))
            };
            (msg, plugin.editor_open)
        } else {
            (
                Message::Plugin(PluginMessage::TogglePluginPanel(pid)),
                self.mixer.selected_plugin == Some(pid),
            )
        };

        let base_color = if is_instrument_slot {
            Color::from_rgb(0.3, 0.75, 0.8)
        } else {
            theme::ACCENT
        };
        let name_color = if is_selected { theme::TEXT } else { base_color };
        let name_btn = button(text(pname).size(9).color(name_color))
            .on_press(click_msg)
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
                            color: base_color,
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

        let remove_msg = match owner {
            PluginOwner::Track(track_id) => Message::Plugin(PluginMessage::RemovePluginFromTrack(track_id, pid)),
            PluginOwner::Bus(bus_id) => Message::Bus(BusMessage::RemovePluginFromBus(bus_id, pid)),
            PluginOwner::Master => Message::Master(MasterMessage::RemovePluginFromMaster(pid)),
        };
        let plugin_del = button(text("\u{00d7}").size(9).color(theme::TEXT_DIM))
            .on_press(remove_msg)
            .style(|_theme, status| theme::small_button_style(status))
            .padding(1);

        row![
            name_btn,
            Space::with_width(Length::Fill),
            plugin_del,
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center)
        .into()
    }

    fn view_channel_strip(&self, track: &TrackState, available_plugins: &[ScannedPlugin]) -> Element<'_, Message> {
        // Sub-tracks get a slimmer strip variant — no FX chain and no
        // input/arm section, because they're fed entirely from their
        // parent plugin's output port.
        let is_sub = track.sub_track.is_some();
        let name_color = if is_sub { theme::TEXT_DIM } else { theme::TEXT };

        // Parent instrument tracks that have at least one sub-track show
        // a small collapse/expand button next to the name. Clicking it
        // toggles `expanded_sub_track_parents`, which view_mixer reads
        // before rendering each sub-track strip.
        let has_sub_tracks = !is_sub
            && self
                .registry
                .tracks
                .iter()
                .any(|t| matches!(t.sub_track, Some(link) if link.parent_track_id == track.id));
        let is_collapsed = has_sub_tracks
            && !self
                .mixer
                .expanded_sub_track_parents
                .contains(&track.id);

        let name_text = text(track.name.clone()).size(13).color(name_color);

        // Save-as-preset button (only for normal tracks, not sub-tracks).
        let save_preset_btn: Option<Element<'_, Message>> = if !is_sub {
            let track_id = track.id;
            Some(
                button(
                    theme::icon(fa::FLOPPY_DISK)
                        .size(9)
                        .color(theme::TEXT_DIM),
                )
                .on_press(Message::Track(TrackMessage::SaveTrackAsPreset(track_id)))
                .padding([2, 3])
                .style(|_theme, status| theme::small_button_style(status))
                .into(),
            )
        } else {
            None
        };

        let track_name: Element<'_, Message> = if has_sub_tracks {
            let glyph = if is_collapsed { "\u{25B8}" } else { "\u{25BE}" };
            let track_id = track.id;
            let toggle = button(text(glyph).size(10).color(theme::TEXT_DIM))
                .on_press(Message::Track(TrackMessage::ToggleSubTracksVisible(track_id)))
                .padding([2, 4])
                .style(|_theme, status| theme::small_button_style(status));
            let mut name_row = row![toggle, name_text]
                .spacing(4)
                .align_y(alignment::Vertical::Center);
            if let Some(btn) = save_preset_btn {
                name_row = name_row
                    .push(Space::with_width(Length::Fill))
                    .push(btn);
            }
            container(name_row)
                .width(Length::Fill)
                .padding([6, 4])
                .into()
        } else {
            let mut name_row = row![name_text]
                .align_y(alignment::Vertical::Center);
            if let Some(btn) = save_preset_btn {
                name_row = name_row
                    .push(Space::with_width(Length::Fill))
                    .push(btn);
            }
            container(name_row)
                .width(Length::Fill)
                .padding([6, 4])
                .into()
        };

        // Same icon vocabulary as the Arrange track header so the two
        // surfaces stay visually consistent.
        let button_row = row![
            mono_button(track.mono, track.id, 12),
            monitor_button(track.monitor_enabled, track.id, 12),
            record_arm_button(track.record_armed, track.id, 12),
            mute_button(track.muted, Message::Track(TrackMessage::ToggleMute(track.id)), 12),
            solo_button(track.soloed, Message::Track(TrackMessage::ToggleSolo(track.id)), 12),
            fx_bypass_button(
                track.fx_bypassed,
                Message::Track(TrackMessage::ToggleTrackFxBypass(track.id)),
                10,
            ),
        ]
        .spacing(4)
        .align_y(alignment::Vertical::Center);

        // Output destination selector: Master + every existing bus.
        let output_picker = self.view_track_output_picker(track);

        // Plugin chain (click to show in bottom panel)
        let mut plugin_section = column![].spacing(2).width(Length::Fill);
        let is_instrument_track = track.track_type == TrackType::Instrument;

        // Sub-tracks are fed by their parent instrument plugin's output
        // port, so they never have their own instrument slot — only an
        // effects chain. Everything else (audio and instrument tracks)
        // follows the existing instrument-slot / FX-slot layout.
        if is_sub {
            // FX slots: every plugin on the sub-track is an effect.
            for plugin in &track.plugins {
                plugin_section = plugin_section.push(
                    self.view_plugin_slot_row(PluginOwner::Track(track.id), plugin, false),
                );
            }
        } else if is_instrument_track {
            if let Some(plugin) = track.plugins.first() {
                plugin_section = plugin_section.push(
                    self.view_plugin_slot_row(PluginOwner::Track(track.id), plugin, true),
                );
            } else if !available_plugins.is_empty() {
                // Empty instrument slot: show a picker filtered to instruments.
                let instruments: Vec<ScannedPlugin> = available_plugins
                    .iter()
                    .filter(|p| p.is_instrument)
                    .cloned()
                    .collect();
                if !instruments.is_empty() {
                    let track_id = track.id;
                    let inst_picker = pick_list(
                        instruments,
                        None::<ScannedPlugin>,
                        move |plugin: ScannedPlugin| Message::Plugin(PluginMessage::AddPluginToTrack(track_id, plugin)),
                    )
                    .placeholder("+ Instrument")
                    .text_size(10)
                    .width(Length::Fill);
                    plugin_section = plugin_section.push(inst_picker);
                } else {
                    plugin_section = plugin_section.push(
                        text("No instruments").size(9).color(theme::TEXT_DIM),
                    );
                }
            }

            // Thin separator between instrument slot and FX chain.
            plugin_section = plugin_section.push(
                container(Space::new(Length::Fill, 1)).style(theme::separator_bg),
            );

            // FX slots: plugins after the instrument.
            for plugin in track.plugins.iter().skip(1) {
                plugin_section = plugin_section.push(
                    self.view_plugin_slot_row(PluginOwner::Track(track.id), plugin, false),
                );
            }
        } else {
            // Audio track: all plugins are FX.
            for plugin in &track.plugins {
                plugin_section = plugin_section.push(
                    self.view_plugin_slot_row(PluginOwner::Track(track.id), plugin, false),
                );
            }
        }

        // FX picker (filtered to effects only). Shown for sub-tracks and
        // audio tracks unconditionally; shown for instrument tracks only
        // once the instrument slot is filled. Extracted from the plugin
        // stack so it can dock directly above the pan knob rather than
        // floating just under the last plugin slot.
        let show_fx_picker = !available_plugins.is_empty()
            && (is_sub || !is_instrument_track || !track.plugins.is_empty());
        let fx_picker_element: Option<Element<'_, Message>> = if show_fx_picker {
            let effects: Vec<ScannedPlugin> = available_plugins
                .iter()
                .filter(|p| !p.is_instrument)
                .cloned()
                .collect();
            if effects.is_empty() {
                None
            } else {
                let track_id = track.id;
                Some(
                    pick_list(
                        effects,
                        None::<ScannedPlugin>,
                        move |plugin: ScannedPlugin| {
                            Message::Plugin(PluginMessage::AddPluginToTrack(track_id, plugin))
                        },
                    )
                    .placeholder("+ FX")
                    .text_size(10)
                    .width(Length::Fill)
                    .into(),
                )
            }
        } else {
            None
        };

        // Pan knob — vertical drag to change, double-click to reset.
        let id = track.id;
        let pan_ctrl = pan_knob(track.pan, move |v| Message::Track(TrackMessage::SetTrackPan(id, v)));
        let pan_label = format_pan(track.pan);
        let pan_row = row![
            text("Pan").size(9).color(theme::TEXT_DIM),
            Space::with_width(Length::Fill),
            pan_ctrl,
            Space::with_width(Length::Fill),
            text(pan_label).size(9).font(Font::MONOSPACE).color(theme::TEXT_DIM),
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center);

        // Dock the +FX picker flush above the pan row so they read as a
        // single block, and rely on `plugin_fill`'s Length::Fill to push
        // that block down to the bottom of the plugin area.
        let fx_pan_block = {
            let mut col = iced::widget::Column::new().spacing(0).width(Length::Fill);
            if let Some(fx) = fx_picker_element {
                col = col.push(fx);
            }
            col.push(pan_row)
        };

        let track_id_for_fader = track.id;
        let fader_block = fader_section(
            track.level_l,
            track.level_r,
            track.volume,
            move |v| Message::Track(TrackMessage::SetTrackVolume(track_id_for_fader, v)),
        );

        // Input device + port picker (when armed). The port dropdown
        // lets the user pick a specific channel on the selected device —
        // critical for multi-input interfaces (e.g. the user's 18-in
        // soundcard). Only shown when the selected device reports a
        // usable channel count.
        let mut bottom_section = column![].spacing(2);
        if track.record_armed && !self.input_devices.is_empty() {
            let selected_device = track
                .input_device_name
                .as_ref()
                .and_then(|name| self.input_devices.iter().find(|d| &d.name == name))
                .cloned();
            let device_channels = selected_device
                .as_ref()
                .map(|d| d.channels)
                .unwrap_or(0);

            let track_id = track.id;
            let device_picker = pick_list(
                self.input_devices.clone(),
                selected_device,
                move |device: InputDeviceInfo| {
                    Message::Track(TrackMessage::SetTrackInputDevice(track_id, Some(device.name)))
                },
            )
            .placeholder("Select input...")
            .text_size(10)
            .width(Length::Fill);

            bottom_section = bottom_section.push(device_picker);

            if device_channels > 0 {
                let is_mono = track.mono;
                // Mono: list every channel 1..=N. Stereo: list every
                // valid pair start (1..=N-1) since the right channel
                // reads `port + 1`.
                let last_valid_index = if is_mono {
                    device_channels
                } else {
                    device_channels.saturating_sub(1)
                };
                let ports: Vec<PortChoice> = (0..last_valid_index)
                    .map(|i| PortChoice {
                        index: i,
                        mono: is_mono,
                    })
                    .collect();
                let selected_port = PortChoice {
                    index: track.input_port_index.min(last_valid_index.saturating_sub(1)),
                    mono: is_mono,
                };
                if !ports.is_empty() {
                    let track_id = track.id;
                    let port_picker = pick_list(
                        ports,
                        Some(selected_port),
                        move |choice: PortChoice| {
                            Message::Track(TrackMessage::SetTrackInputPort(track_id, choice.index))
                        },
                    )
                    .text_size(10)
                    .width(Length::Fill);
                    bottom_section = bottom_section.push(port_picker);
                }
            }
        }

        let bg = if track.record_armed {
            theme::PANEL_ARMED
        } else if is_sub {
            theme::PANEL
        } else {
            theme::PANEL_DARK
        };
        let border_color = if track.record_armed { theme::RECORD_RED } else { theme::SEPARATOR };

        // The plugin section renders for every strip — sub-tracks show an
        // effects-only chain, instrument tracks show the instrument slot +
        // FX chain, audio tracks show an FX chain. All three paths are
        // constructed above into `plugin_section`.
        let plugin_fill: Element<'_, Message> = container(plugin_section)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(alignment::Vertical::Top)
            .into();

        let strip_style = move |_theme: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(bg)),
            border: iced::Border {
                color: border_color,
                width: 0.5,
                radius: 0.0.into(),
            },
            ..Default::default()
        };

        if is_collapsed {
            // Two-column layout: normal controls on the left, compact
            // subtrack meters on the right.
            let left_col = column![
                track_name,
                button_row,
                plugin_fill,
                fx_pan_block,
                fader_block,
                output_picker,
                bottom_section,
            ]
            .spacing(4)
            .width(theme::MIXER_STRIP_WIDTH - 12)
            .height(Length::Fill);

            let v_sep = container(Space::new(1, Length::Fill))
                .style(theme::separator_bg);

            let right_col = self.view_collapsed_subtrack_meters(track.id);

            let strip_content = row![left_col, v_sep, right_col]
                .height(Length::Fill)
                .padding(6)
                .width(theme::MIXER_STRIP_WIDTH * 2);

            container(strip_content)
                .height(Length::Fill)
                .style(strip_style)
                .into()
        } else {
            let strip_content = column![
                track_name,
                button_row,
                plugin_fill,
                fx_pan_block,
                fader_block,
                output_picker,
                bottom_section,
            ]
            .spacing(4)
            .padding(6)
            .width(theme::MIXER_STRIP_WIDTH)
            .height(Length::Fill);

            container(strip_content)
                .height(Length::Fill)
                .style(strip_style)
                .into()
        }
    }

    /// Compact subtrack meters shown in the right half of a collapsed
    /// parent strip. Each subtrack gets a name label and stereo meter bars.
    fn view_collapsed_subtrack_meters(&self, parent_id: TrackId) -> Element<'_, Message> {
        let mut subtracks: Vec<&TrackState> = self
            .registry
            .tracks
            .iter()
            .filter(|t| matches!(t.sub_track, Some(link) if link.parent_track_id == parent_id))
            .collect();
        subtracks.sort_by_key(|t| t.order);

        let mut meters_row = row![].spacing(6);
        for sub in subtracks {
            let name: String = if sub.name.chars().count() > 5 {
                let mut s: String = sub.name.chars().take(4).collect();
                s.push('.');
                s
            } else {
                sub.name.clone()
            };
            let name_label = text(name).size(8).color(theme::TEXT_DIM);
            let meter = meter_v(sub.level_l, sub.level_r, theme::FADER_HEIGHT);
            let col = column![name_label, meter]
                .spacing(2)
                .align_x(alignment::Horizontal::Center);
            meters_row = meters_row.push(col);
        }

        column![
            Space::with_height(Length::Fill),
            meters_row,
            Space::with_height(24),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    /// Pick-list of available output destinations (Master + all busses)
    /// for a given track. Emits `Message::Track(TrackMessage::SetTrackOutput)` when changed.
    fn view_track_output_picker(&self, track: &TrackState) -> Element<'_, Message> {
        let mut choices: Vec<OutputChoice> = Vec::with_capacity(1 + self.registry.busses.len());
        choices.push(OutputChoice {
            label: "→ Master".to_string(),
            output: TrackOutput::Master,
        });
        for bus in self.sorted_busses() {
            choices.push(OutputChoice {
                label: format!("→ {}", bus.name),
                output: TrackOutput::Bus(bus.id),
            });
        }

        // Resolve the currently-selected choice (fall back to Master if the
        // track's bus id isn't in the choice list — e.g. mid-remove).
        let selected = choices
            .iter()
            .find(|c| c.output == track.output)
            .cloned()
            .unwrap_or_else(|| choices[0].clone());

        let track_id = track.id;
        let picker = pick_list(choices, Some(selected), move |choice: OutputChoice| {
            Message::Track(TrackMessage::SetTrackOutput(track_id, choice.output))
        })
        .text_size(9)
        .width(Length::Fill);

        container(picker)
            .width(Length::Fill)
            .into()
    }

    /// Render a bus channel strip. Structurally similar to a track strip
    /// but trimmed: no mono/monitor/arm, no instrument slot, no input
    /// device picker, no output selector (busses always go to master).
    fn view_bus_strip(
        &self,
        bus: &BusState,
        available_plugins: &[ScannedPlugin],
    ) -> Element<'_, Message> {
        let bus_name = container(
            text(bus.name.clone()).size(13).color(theme::TEXT),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .padding([6, 4]);

        // Mute + FX bypass + Remove buttons — same icons as the track header.
        let bus_id = bus.id;
        let button_row = row![
            mute_button(bus.muted, Message::Bus(BusMessage::ToggleBusMute(bus_id)), 12),
            fx_bypass_button(
                bus.fx_bypassed,
                Message::Bus(BusMessage::ToggleBusFxBypass(bus_id)),
                10,
            ),
            Space::with_width(Length::Fill),
            bus_remove_button(bus_id, 12),
        ]
        .spacing(4)
        .align_y(alignment::Vertical::Center);

        // Plugin chain (all effects — no instrument slot on busses).
        let mut plugin_section = column![].spacing(2).width(Length::Fill);
        for plugin in &bus.plugins {
            plugin_section = plugin_section.push(
                self.view_plugin_slot_row(PluginOwner::Bus(bus_id), plugin, false),
            );
        }

        // Extract the +FX picker so it can dock above the pan knob (same
        // treatment as track strips).
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
                    pick_list(
                        effects,
                        None::<ScannedPlugin>,
                        move |plugin: ScannedPlugin| Message::Bus(BusMessage::AddPluginToBus(bus_id, plugin)),
                    )
                    .placeholder("+ FX")
                    .text_size(10)
                    .width(Length::Fill)
                    .into(),
                )
            }
        };

        // Pan knob — vertical drag to change, double-click to reset.
        let pan_ctrl = pan_knob(bus.pan, move |v| Message::Bus(BusMessage::SetBusPan(bus_id, v)));
        let pan_label = format_pan(bus.pan);
        let pan_row = row![
            text("Pan").size(9).color(theme::TEXT_DIM),
            Space::with_width(Length::Fill),
            pan_ctrl,
            Space::with_width(Length::Fill),
            text(pan_label).size(9).font(Font::MONOSPACE).color(theme::TEXT_DIM),
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

        // FX section absorbs all slack (same treatment as track strips).
        let plugin_fill = container(plugin_section)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(alignment::Vertical::Top);

        let strip_content = column![
            bus_name,
            button_row,
            plugin_fill,
            fx_pan_block,
            fader_block,
        ]
        .spacing(4)
        .padding(6)
        .width(theme::MIXER_STRIP_WIDTH)
        .height(Length::Fill);

        container(strip_content)
            .height(Length::Fill)
            .style(theme::panel_dark_outlined)
            .into()
    }

    fn view_master_strip(&self, available_plugins: &[ScannedPlugin]) -> Element<'_, Message> {
        let label = container(
            text("Master").size(14).color(theme::ACCENT),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .padding([6, 4]);

        // FX bypass button, centered in its own row so the master strip
        // has a dedicated control spot (tracks and busses share a row
        // with other toggles; the master strip only has this one).
        let button_row = container(
            fx_bypass_button(
                self.master_fx_bypassed,
                Message::Master(MasterMessage::ToggleMasterFxBypass),
                10,
            ),
        )
        .width(Length::Fill)
        .center_x(Length::Fill);

        // Plugin chain — every plugin is an effect.
        let mut plugin_section = column![].spacing(2).width(Length::Fill);
        for plugin in &self.master_plugins {
            plugin_section = plugin_section
                .push(self.view_plugin_slot_row(PluginOwner::Master, plugin, false));
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
                    pick_list(
                        effects,
                        None::<ScannedPlugin>,
                        |plugin: ScannedPlugin| {
                            Message::Master(MasterMessage::AddPluginToMaster(plugin))
                        },
                    )
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
            .align_y(alignment::Vertical::Top);

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

    /// Bottom panel showing the selected plugin's UI.
    fn view_plugin_panel(&self) -> Option<Element<'_, Message>> {
        let selected_id = self.mixer.selected_plugin?;

        // Find the plugin across all tracks, busses, and the master chain.
        let plugin = self.registry.tracks.iter()
            .flat_map(|t| t.plugins.iter())
            .chain(self.registry.busses.iter().flat_map(|b| b.plugins.iter()))
            .chain(self.master_plugins.iter())
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
            PluginCustomState::Generic => {
                resonance_plugin::ui::view_generic_params(&ui_params)
            }
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
                .on_press(Message::Plugin(PluginMessage::TogglePluginPanel(selected_id)))
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
        .style(theme::panel_bg);

        Some(panel.into())
    }
}
