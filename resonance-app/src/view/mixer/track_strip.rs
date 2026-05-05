//! Per-track channel strip (and the collapsed-parent meters and the
//! output-destination picker that hangs off each strip).

use iced::widget::text::Shaping;
use iced::widget::{button, column, container, pick_list, row, text, Space};
use iced::{alignment, Element, Font, Length};
use resonance_audio::types::*;

use crate::message::*;
use crate::state::*;
use crate::theme::{self, fa};
use crate::util::format_pan;
use crate::view::controls::{
    fader_section, fx_bypass_button, meter_v, monitor_button, mono_button, mute_button,
    record_arm_button, solo_button,
};
use crate::view::knob::pan_knob;

use super::picks::{
    input_channel_choices, midi_choices, output_channel_choices, MidiChannelChoice,
    MidiPickerChoice, OutputChoice, PluginOwner, PortChoice,
};

impl crate::Resonance {
    pub(super) fn view_channel_strip(
        &self,
        track: &TrackState,
        available_plugins: &[ScannedPlugin],
    ) -> Element<'_, Message> {
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
        let is_collapsed =
            has_sub_tracks && !self.mixer.expanded_sub_track_parents.contains(&track.id);

        let name_text = text(track.name.clone()).size(13).color(name_color);

        let track_name: Element<'_, Message> = if has_sub_tracks {
            let glyph = if is_collapsed {
                fa::CARET_RIGHT
            } else {
                fa::CARET_DOWN
            };
            let track_id = track.id;
            let toggle = button(theme::icon(glyph).size(10).color(theme::TEXT_DIM))
                .on_press(Message::Track(TrackMessage::ToggleSubTracksVisible(
                    track_id,
                )))
                .padding([2, 4])
                .style(|_theme, status| theme::small_button_style(status));
            let name_row = row![toggle, name_text]
                .spacing(4)
                .align_y(alignment::Vertical::Center);
            container(name_row)
                .width(Length::Fill)
                .padding([6, 4])
                .into()
        } else {
            let name_row = row![name_text].align_y(alignment::Vertical::Center);
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
            mute_button(
                track.muted,
                Message::Track(TrackMessage::ToggleMute(track.id)),
                12
            ),
            solo_button(
                track.soloed,
                Message::Track(TrackMessage::ToggleSolo(track.id)),
                12
            ),
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
            for plugin in &track.plugins {
                plugin_section = plugin_section.push(self.view_plugin_slot_row(
                    PluginOwner::Track(track.id),
                    plugin,
                    false,
                ));
            }
        } else if is_instrument_track {
            if let Some(plugin) = track.plugins.first() {
                plugin_section = plugin_section.push(self.view_plugin_slot_row(
                    PluginOwner::Track(track.id),
                    plugin,
                    true,
                ));
            } else if !available_plugins.is_empty() {
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
                        move |plugin: ScannedPlugin| {
                            Message::Plugin(PluginMessage::AddPluginToTrack(track_id, plugin))
                        },
                    )
                    .placeholder("+ Instrument")
                    .text_size(10)
                    .width(Length::Fill);
                    plugin_section = plugin_section.push(inst_picker);
                } else {
                    plugin_section =
                        plugin_section.push(text("No instruments").size(9).color(theme::TEXT_DIM));
                }
            }

            // Thin separator between instrument slot and FX chain.
            plugin_section = plugin_section
                .push(container(Space::new(Length::Fill, 1)).style(theme::separator_bg));

            // FX slots: plugins after the instrument.
            for plugin in track.plugins.iter().skip(1) {
                plugin_section = plugin_section.push(self.view_plugin_slot_row(
                    PluginOwner::Track(track.id),
                    plugin,
                    false,
                ));
            }
        } else {
            // Audio track: all plugins are FX.
            for plugin in &track.plugins {
                plugin_section = plugin_section.push(self.view_plugin_slot_row(
                    PluginOwner::Track(track.id),
                    plugin,
                    false,
                ));
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
        let pan_ctrl = pan_knob(track.pan, move |v| {
            Message::Track(TrackMessage::SetTrackPan(id, v))
        });
        let pan_label = format_pan(track.pan);
        let pan_row = row![
            text("Pan").size(9).color(theme::TEXT_DIM),
            Space::with_width(Length::Fill),
            pan_ctrl,
            Space::with_width(Length::Fill),
            text(pan_label)
                .size(9)
                .font(Font::MONOSPACE)
                .color(theme::TEXT_DIM),
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center);

        // Dock the +FX picker flush above the pan row so they read as a
        // single block, and rely on `plugin_fill`'s Length::Fill to
        // push that block down to the bottom of the plugin area.
        let fx_pan_block = {
            let mut col = iced::widget::Column::new().spacing(0).width(Length::Fill);
            if let Some(fx) = fx_picker_element {
                col = col.push(fx);
            }
            col.push(pan_row)
        };

        let track_id_for_fader = track.id;
        let fader_block = fader_section(track.level_l, track.level_r, track.volume, move |v| {
            Message::Track(TrackMessage::SetTrackVolume(track_id_for_fader, v))
        });

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
            let device_channels = selected_device.as_ref().map(|d| d.channels).unwrap_or(0);

            let track_id = track.id;
            let device_picker = pick_list(
                self.input_devices.clone(),
                selected_device,
                move |device: InputDeviceInfo| {
                    Message::Track(TrackMessage::SetTrackInputDevice(
                        track_id,
                        Some(device.name),
                    ))
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
                    index: track
                        .input_port_index
                        .min(last_valid_index.saturating_sub(1)),
                    mono: is_mono,
                };
                if !ports.is_empty() {
                    let track_id = track.id;
                    let port_picker =
                        pick_list(ports, Some(selected_port), move |choice: PortChoice| {
                            Message::Track(TrackMessage::SetTrackInputPort(track_id, choice.index))
                        })
                        .text_size(10)
                        .width(Length::Fill);
                    bottom_section = bottom_section.push(port_picker);
                }
            }
        }

        // MIDI in / MIDI out pickers — instrument tracks only.
        // Always visible (independent of record_armed) so a controller
        // can be wired up for live play without arming the track.
        // Sub-tracks share their parent's MIDI routing, so they don't
        // get pickers of their own.
        if track.track_type == TrackType::Instrument && track.sub_track.is_none() {
            let track_id = track.id;
            let in_choices = midi_choices(
                &self.midi_input_devices,
                track.midi_input_device.as_deref(),
            );
            let in_selected = MidiPickerChoice(track.midi_input_device.clone());
            let in_picker = pick_list(in_choices, Some(in_selected), move |choice| {
                Message::Track(TrackMessage::SetTrackMidiInputDevice(track_id, choice.0))
            })
            .placeholder("MIDI in...")
            .text_size(10)
            .width(Length::Fill);
            bottom_section = bottom_section.push(in_picker);

            // Show the input channel picker only when an input device
            // is configured — pointless otherwise, and removing it
            // saves vertical space on every other instrument strip.
            if track.midi_input_device.is_some() {
                let in_ch_picker = pick_list(
                    input_channel_choices(),
                    Some(MidiChannelChoice(track.midi_input_channel)),
                    move |choice| {
                        Message::Track(TrackMessage::SetTrackMidiInputChannel(track_id, choice.0))
                    },
                )
                .text_size(10)
                .width(Length::Fill);
                bottom_section = bottom_section.push(in_ch_picker);
            }

            let out_choices = midi_choices(
                &self.midi_output_devices,
                track.midi_output_device.as_deref(),
            );
            let out_selected = MidiPickerChoice(track.midi_output_device.clone());
            let out_picker = pick_list(out_choices, Some(out_selected), move |choice| {
                Message::Track(TrackMessage::SetTrackMidiOutputDevice(track_id, choice.0))
            })
            .placeholder("MIDI out...")
            .text_size(10)
            .width(Length::Fill);
            bottom_section = bottom_section.push(out_picker);

            if track.midi_output_device.is_some() {
                // Outputs always emit on a specific channel — there is
                // no "Omni" semantics. Default to channel 1 when unset
                // on the engine side, and show "Ch 1" here.
                let selected =
                    MidiChannelChoice(Some(track.midi_output_channel.unwrap_or(0)));
                let out_ch_picker = pick_list(
                    output_channel_choices(),
                    Some(selected),
                    move |choice| {
                        Message::Track(TrackMessage::SetTrackMidiOutputChannel(
                            track_id, choice.0,
                        ))
                    },
                )
                .text_size(10)
                .width(Length::Fill);
                bottom_section = bottom_section.push(out_ch_picker);
            }
        }

        let bg = if track.record_armed {
            theme::PANEL_ARMED
        } else if is_sub {
            theme::PANEL
        } else {
            theme::PANEL_DARK
        };
        let border_color = if track.record_armed {
            theme::RECORD_RED
        } else {
            theme::SEPARATOR
        };

        // The plugin section renders for every strip — sub-tracks show
        // an effects-only chain, instrument tracks show the instrument
        // slot + FX chain, audio tracks show an FX chain. All three
        // paths are constructed above into `plugin_section`.
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

            let v_sep = container(Space::new(1, Length::Fill)).style(theme::separator_bg);

            let right_col =
                container(self.view_collapsed_subtrack_meters(track.id)).padding([0, 4]);

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

        let mut meters_row = row![].spacing(4);
        for sub in &subtracks {
            // Show the port label (after "→") rather than the full name,
            // so "Instrument 2 → Kick" displays as "Kick" instead of "Inst.".
            let short_name = sub.name.split(" \u{2192} ").nth(1).unwrap_or(&sub.name);
            let label: String = if short_name.chars().count() > 6 {
                let mut s: String = short_name.chars().take(5).collect();
                s.push('.');
                s
            } else {
                short_name.to_string()
            };
            let name_label = text(label).size(9).color(theme::TEXT_DIM);
            let meter = meter_v(sub.level_l, sub.level_r, theme::FADER_HEIGHT);
            let col = column![name_label, meter]
                .spacing(2)
                .align_x(alignment::Horizontal::Center);
            meters_row = meters_row.push(col);
        }

        let title = text(format!("{} outs", subtracks.len()))
            .size(9)
            .color(theme::TEXT_DIM);

        column![
            title,
            Space::with_height(Length::Fill),
            meters_row,
            Space::with_height(24),
        ]
        .spacing(2)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    /// Pick-list of available output destinations (Master + all busses)
    /// for a given track. Emits `Message::Track(TrackMessage::SetTrackOutput)` when changed.
    fn view_track_output_picker(&self, track: &TrackState) -> Element<'_, Message> {
        let mut choices: Vec<OutputChoice> = Vec::with_capacity(1 + self.registry.busses.len());
        choices.push(OutputChoice {
            label: format!("{} Master", fa::ARROW_RIGHT),
            output: TrackOutput::Master,
        });
        for bus in self.sorted_busses() {
            choices.push(OutputChoice {
                label: format!("{} {}", fa::ARROW_RIGHT, bus.name),
                output: TrackOutput::Bus(bus.id),
            });
        }

        // Resolve the currently-selected choice (fall back to Master if
        // the track's bus id isn't in the choice list — e.g. mid-remove).
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
        .text_shaping(Shaping::Advanced)
        .width(Length::Fill);

        container(picker).width(Length::Fill).into()
    }
}
