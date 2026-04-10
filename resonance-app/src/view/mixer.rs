/// Mixer view rendering for the Resonance application.
use crate::message::Message;
use crate::state::*;
use crate::theme;
use crate::theme::fa;
use crate::util::{format_db, format_pan};
use iced::widget::{
    button, column, container, pick_list, row, scrollable, slider, text,
    vertical_slider, Space,
};
use iced::{alignment, Color, Element, Font, Length};
use resonance_audio::types::*;

/// Which container a plugin slot belongs to. Used so `view_plugin_slot_row`
/// can emit the right remove message regardless of whether it's rendering
/// a track's plugin or a bus's plugin.
#[derive(Debug, Clone, Copy)]
enum PluginOwner {
    Track(TrackId),
    Bus(BusId),
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

/// Convert linear amplitude to meter bar height (logarithmic/dB scale).
fn level_to_bar_height(level: f32, max_height: f32) -> f32 {
    if level < 0.0001 {
        return 0.0;
    }
    let db = 20.0 * level.log10();
    let normalized = (db + 60.0) / 66.0; // -60dB=0, +6dB=1
    normalized.clamp(0.0, 1.0) * max_height
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

/// Render a single vertical VU meter bar (bottom-aligned colored bar on dark bg).
fn meter_bar_v<'a>(level: f32, max_height: f32) -> Element<'a, Message> {
    let bar_height = level_to_bar_height(level, max_height);
    let color = level_color(level);

    // Spacer pushes the colored bar to the bottom
    let spacer_height = (max_height - bar_height).max(0.0);

    let bar = container(Space::new(0.0, 0.0))
        .width(Length::Fill)
        .height(bar_height)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(color)),
            ..Default::default()
        });

    container(
        column![
            Space::new(Length::Fill, spacer_height),
            bar,
        ],
    )
    .width(6)
    .height(max_height)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::METER_BG)),
        ..Default::default()
    })
    .into()
}

/// Render a stereo vertical VU meter (L + R bars side by side).
fn view_meter_v<'a>(level_l: f32, level_r: f32, height: f32) -> Element<'a, Message> {
    row![meter_bar_v(level_l, height), meter_bar_v(level_r, height)]
        .spacing(1)
        .into()
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
                if self
                    .collapsed_sub_track_parents
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
        let master_strip = self.view_master_strip();
        let v_separator_tracks =
            container(Space::new(1, Length::Fill)).style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::SEPARATOR)),
                ..Default::default()
            });
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
            container(Space::new(1, Length::Fill)).style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::SEPARATOR)),
                ..Default::default()
            });
        let busses_area = row![scrollable_busses, v_separator_busses, add_bus_strip]
            .height(Length::FillPortion(1));

        let h_sep_mid = container(Space::new(Length::Fill, 1)).style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::SEPARATOR)),
            ..Default::default()
        });

        let mut mixer_col = column![].spacing(0);
        mixer_col = mixer_col.push(tracks_area);
        mixer_col = mixer_col.push(h_sep_mid);
        mixer_col = mixer_col.push(busses_area);

        if let Some(panel) = self.view_plugin_panel() {
            let h_sep = container(Space::new(Length::Fill, 1)).style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::SEPARATOR)),
                ..Default::default()
            });
            mixer_col = mixer_col.push(h_sep);
            mixer_col = mixer_col.push(panel);
        }

        container(mixer_col)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::BG)),
                ..Default::default()
            })
            .into()
    }

    /// Small "+ Bus" strip that lives in the same slot the master strip
    /// occupies in the top row, but in the bus row. Clicking it dispatches
    /// `Message::AddBus`.
    fn view_add_bus_strip(&self) -> Element<'_, Message> {
        let label = container(
            text("Busses").size(11).color(theme::TEXT_DIM),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .padding([6, 4]);

        let add_btn = button(text("+ Bus").size(11).color(theme::TEXT))
            .on_press(Message::AddBus)
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
        let is_selected = self.selected_plugin == Some(pid);

        let base_color = if is_instrument_slot {
            Color::from_rgb(0.3, 0.75, 0.8)
        } else {
            theme::ACCENT
        };
        let name_color = if is_selected { theme::TEXT } else { base_color };
        let name_btn = button(text(pname).size(9).color(name_color))
            .on_press(Message::TogglePluginPanel(pid))
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
            PluginOwner::Track(track_id) => Message::RemovePluginFromTrack(track_id, pid),
            PluginOwner::Bus(bus_id) => Message::RemovePluginFromBus(bus_id, pid),
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
        // toggles `collapsed_sub_track_parents`, which view_mixer reads
        // before rendering each sub-track strip.
        let has_sub_tracks = !is_sub
            && self
                .tracks
                .iter()
                .any(|t| matches!(t.sub_track, Some(link) if link.parent_track_id == track.id));
        let is_collapsed = self
            .collapsed_sub_track_parents
            .contains(&track.id);

        let name_text = text(track.name.clone()).size(13).color(name_color);
        let track_name: Element<'_, Message> = if has_sub_tracks {
            let glyph = if is_collapsed { "\u{25B8}" } else { "\u{25BE}" };
            let track_id = track.id;
            let toggle = button(text(glyph).size(10).color(theme::TEXT_DIM))
                .on_press(Message::ToggleSubTracksVisible(track_id))
                .padding([2, 4])
                .style(|_theme, status| theme::small_button_style(status));
            container(
                row![toggle, name_text]
                    .spacing(4)
                    .align_y(alignment::Vertical::Center),
            )
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding([6, 4])
            .into()
        } else {
            container(name_text)
                .width(Length::Fill)
                .center_x(Length::Fill)
                .padding([6, 4])
                .into()
        };

        // Same icon vocabulary as the Arrange track header so the two
        // surfaces stay visually consistent.
        let rec_color = if track.record_armed { theme::RECORD_RED } else { theme::TEXT_DIM };
        let armed = track.record_armed;
        let rec_btn = button(theme::icon(fa::CIRCLE).size(12).color(rec_color))
            .on_press(Message::ToggleRecordArm(track.id))
            .style(move |_theme, status| {
                if armed { theme::record_armed_button_style(status) }
                else { theme::small_button_style(status) }
            })
            .padding(2);

        let mute_color = if track.muted { theme::ACCENT } else { theme::TEXT_DIM };
        let mute_btn = button(theme::icon(fa::VOLUME_XMARK).size(12).color(mute_color))
            .on_press(Message::ToggleMute(track.id))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2);

        let solo_color = if track.soloed { theme::SOLO_YELLOW } else { theme::TEXT_DIM };
        let solo_btn = button(theme::icon(fa::HEADPHONES).size(12).color(solo_color))
            .on_press(Message::ToggleSolo(track.id))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2);

        let mon_color = if track.monitor_enabled { theme::METRONOME_ON } else { theme::TEXT_DIM };
        let mon_enabled = track.monitor_enabled;
        let mon_btn = button(theme::icon(fa::EYE).size(12).color(mon_color))
            .on_press(Message::ToggleMonitor(track.id))
            .style(move |_theme, status| {
                theme::toggle_button_style(mon_enabled, theme::METRONOME_ON, true, status)
            })
            .padding(2);

        let is_mono = track.mono;
        let mono_glyph = if is_mono { fa::CIRCLE_HOLLOW } else { fa::CIRCLE_HOLLOW_DOUBLE };
        let mono_btn = button(theme::icon(mono_glyph).size(12).color(theme::TEXT))
            .on_press(Message::ToggleTrackMono(track.id))
            .style(move |_theme, status| {
                theme::mono_button_style(is_mono, status)
            })
            .padding(2);

        let button_row = row![mono_btn, mon_btn, rec_btn, mute_btn, solo_btn]
            .spacing(4)
            .align_y(alignment::Vertical::Center);

        // Output destination selector: Master + every existing bus.
        let output_picker = self.view_track_output_picker(track);

        // Plugin chain (click to show in bottom panel)
        let mut plugin_section = column![].spacing(2).width(Length::Fill);
        let is_instrument_track = track.track_type == TrackType::Instrument;

        // For instrument tracks, the first plugin is the dedicated instrument slot.
        if is_instrument_track {
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
                        move |plugin: ScannedPlugin| Message::AddPluginToTrack(track_id, plugin),
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
                container(Space::new(Length::Fill, 1)).style(|_theme| container::Style {
                    background: Some(iced::Background::Color(theme::SEPARATOR)),
                    ..Default::default()
                }),
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

        // FX picker (filtered to effects only). Only shown for instrument tracks
        // once the instrument slot is filled.
        let show_fx_picker = !available_plugins.is_empty()
            && (!is_instrument_track || !track.plugins.is_empty());
        if show_fx_picker {
            let effects: Vec<ScannedPlugin> = available_plugins
                .iter()
                .filter(|p| !p.is_instrument)
                .cloned()
                .collect();
            if !effects.is_empty() {
                let track_id = track.id;
                let fx_picker = pick_list(
                    effects,
                    None::<ScannedPlugin>,
                    move |plugin: ScannedPlugin| Message::AddPluginToTrack(track_id, plugin),
                )
                .placeholder("+ FX")
                .text_size(10)
                .width(Length::Fill);
                plugin_section = plugin_section.push(fx_picker);
            }
        }

        // Pan control (horizontal)
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

        // Volume fader (vertical) + VU meters
        let fader_height = 120.0;

        let vol_fader = vertical_slider(-60.0..=6.0f32, track.volume, {
            let id = track.id;
            move |v| Message::SetTrackVolume(id, v)
        })
        .height(fader_height)
        .step(0.1);

        let meters = view_meter_v(track.level_l, track.level_r, fader_height);

        let vol_label = format_db(track.volume);
        let fader_row = row![
            meters,
            vol_fader,
        ]
        .spacing(4)
        .align_y(alignment::Vertical::Center);

        let fader_section = column![
            container(fader_row)
                .width(Length::Fill)
                .center_x(Length::Fill),
            text(vol_label).size(9).font(Font::MONOSPACE).color(theme::TEXT_DIM),
        ]
        .spacing(2)
        .align_x(alignment::Horizontal::Center);

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
                    Message::SetTrackInputDevice(track_id, Some(device.name))
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
                            Message::SetTrackInputPort(track_id, choice.index)
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

        // Sub-tracks skip the plugin chain entirely — they're fed by the
        // parent plugin's output port, not their own chain. Use a spacer
        // to keep the strip height consistent with parent tracks so the
        // faders still line up visually.
        let plugin_fill: Element<'_, Message> = if is_sub {
            container(Space::new(Length::Fill, Length::Fill))
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            container(plugin_section)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(alignment::Vertical::Top)
                .into()
        };

        let strip_content = column![
            track_name,
            button_row,
            plugin_fill,
            pan_row,
            fader_section,
            output_picker,
            bottom_section,
        ]
        .spacing(4)
        .padding(6)
        .width(theme::MIXER_STRIP_WIDTH)
        .height(Length::Fill);

        container(strip_content)
            .height(Length::Fill)
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

    /// Pick-list of available output destinations (Master + all busses)
    /// for a given track. Emits `Message::SetTrackOutput` when changed.
    fn view_track_output_picker(&self, track: &TrackState) -> Element<'_, Message> {
        let mut choices: Vec<OutputChoice> = Vec::with_capacity(1 + self.busses.len());
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
            Message::SetTrackOutput(track_id, choice.output)
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

        // Mute + Remove buttons — same icons as the track header.
        let mute_color = if bus.muted { theme::ACCENT } else { theme::TEXT_DIM };
        let bus_id = bus.id;
        let mute_btn = button(theme::icon(fa::VOLUME_XMARK).size(12).color(mute_color))
            .on_press(Message::ToggleBusMute(bus_id))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2);

        let remove_btn = button(theme::icon(fa::TRASH).size(12).color(theme::TEXT_DIM))
            .on_press(Message::RemoveBus(bus_id))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2);

        let button_row = row![mute_btn, Space::with_width(Length::Fill), remove_btn]
            .spacing(4)
            .align_y(alignment::Vertical::Center);

        // Plugin chain (all effects — no instrument slot on busses).
        let mut plugin_section = column![].spacing(2).width(Length::Fill);
        for plugin in &bus.plugins {
            plugin_section = plugin_section.push(
                self.view_plugin_slot_row(PluginOwner::Bus(bus_id), plugin, false),
            );
        }

        if !available_plugins.is_empty() {
            let effects: Vec<ScannedPlugin> = available_plugins
                .iter()
                .filter(|p| !p.is_instrument)
                .cloned()
                .collect();
            if !effects.is_empty() {
                let fx_picker = pick_list(
                    effects,
                    None::<ScannedPlugin>,
                    move |plugin: ScannedPlugin| Message::AddPluginToBus(bus_id, plugin),
                )
                .placeholder("+ FX")
                .text_size(10)
                .width(Length::Fill);
                plugin_section = plugin_section.push(fx_picker);
            }
        }

        // Pan control.
        let pan_slider = slider(-1.0..=1.0f32, bus.pan, move |v| {
            Message::SetBusPan(bus_id, v)
        })
        .width(Length::Fill)
        .step(0.01);

        let pan_label = format_pan(bus.pan);
        let pan_row = row![
            text("Pan").size(9).color(theme::TEXT_DIM),
            Space::with_width(4),
            pan_slider,
            Space::with_width(4),
            text(pan_label).size(9).font(Font::MONOSPACE).color(theme::TEXT_DIM),
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center);

        // Volume fader + meters.
        let fader_height = 120.0;
        let vol_fader = vertical_slider(-60.0..=6.0f32, bus.volume, move |v| {
            Message::SetBusVolume(bus_id, v)
        })
        .height(fader_height)
        .step(0.1);
        let meters = view_meter_v(bus.level_l, bus.level_r, fader_height);
        let vol_label = format_db(bus.volume);
        let fader_row = row![meters, vol_fader]
            .spacing(4)
            .align_y(alignment::Vertical::Center);
        let fader_section = column![
            container(fader_row)
                .width(Length::Fill)
                .center_x(Length::Fill),
            text(vol_label).size(9).font(Font::MONOSPACE).color(theme::TEXT_DIM),
        ]
        .spacing(2)
        .align_x(alignment::Horizontal::Center);

        // FX section absorbs all slack (same treatment as track strips).
        let plugin_fill = container(plugin_section)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(alignment::Vertical::Top);

        let strip_content = column![
            bus_name,
            button_row,
            plugin_fill,
            pan_row,
            fader_section,
        ]
        .spacing(4)
        .padding(6)
        .width(theme::MIXER_STRIP_WIDTH)
        .height(Length::Fill);

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

    fn view_master_strip(&self) -> Element<'_, Message> {
        let label = container(
            text("Master").size(14).color(theme::ACCENT),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .padding([6, 4]);

        let fader_height = 120.0;

        let vol_fader = vertical_slider(-60.0..=6.0f32, self.master_volume, Message::SetMasterVolume)
            .height(fader_height)
            .step(0.1);

        let meters = view_meter_v(self.master_level_l, self.master_level_r, fader_height);

        let vol_label = format_db(self.master_volume);

        let fader_row = row![
            meters,
            vol_fader,
        ]
        .spacing(4)
        .align_y(alignment::Vertical::Center);

        let fader_section = column![
            container(fader_row)
                .width(Length::Fill)
                .center_x(Length::Fill),
            text(vol_label).size(9).font(Font::MONOSPACE).color(theme::TEXT_DIM),
        ]
        .spacing(2)
        .align_x(alignment::Horizontal::Center);

        let bounce_btn: Element<'_, Message> = if self.bouncing {
            text("Bouncing...").size(8).color(theme::ACCENT).into()
        } else {
            button(text("Bounce").size(8).color(theme::TEXT))
                .on_press(Message::BounceToWav)
                .style(|_theme, status| theme::small_button_style(status))
                .padding([2, 8])
                .into()
        };

        let bounce_row = container(bounce_btn)
            .width(Length::Fill)
            .center_x(Length::Fill);

        let strip_content = column![
            label,
            bounce_row,
            Space::with_height(Length::Fill),
            fader_section,
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

    /// Bottom panel showing the selected plugin's UI.
    fn view_plugin_panel(&self) -> Option<Element<'_, Message>> {
        let selected_id = self.selected_plugin?;

        // Find the plugin across all tracks and busses.
        let plugin = self.tracks.iter()
            .flat_map(|t| t.plugins.iter())
            .chain(self.busses.iter().flat_map(|b| b.plugins.iter()))
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
            PluginCustomState::Amp(state) => {
                resonance_amp::ui::view(state, &ui_params)
            }
            PluginCustomState::Ir(state) => {
                resonance_ir::ui::view(state, &ui_params)
            }
            PluginCustomState::Generic => {
                resonance_plugin::ui::view_generic_params(&ui_params)
            }
        };

        let inst_id = selected_id;
        let mapped = plugin_element.map(move |event| {
            use resonance_plugin::ui::PluginUiEvent;
            match event {
                PluginUiEvent::SetParam(param_id, value) => Message::SetPluginParam(inst_id, param_id, value),
                PluginUiEvent::BrowseFile => Message::PluginBrowseFile(inst_id),
                PluginUiEvent::PrevFile => Message::PluginPrevFile(inst_id),
                PluginUiEvent::NextFile => Message::PluginNextFile(inst_id),
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
                Message::ClosePluginEditor(selected_id)
            } else {
                Message::OpenPluginEditor(selected_id)
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
                .on_press(Message::TogglePluginPanel(selected_id))
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
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::PANEL)),
            ..Default::default()
        });

        Some(panel.into())
    }
}
