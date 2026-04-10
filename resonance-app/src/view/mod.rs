/// View rendering for the Resonance application.
pub(crate) mod mixer;

use crate::message::Message;
use crate::midi_editor::PianoRollCanvas;
use crate::state::*;
use crate::theme;
use crate::timeline::TimelineCanvas;
use crate::util::{format_db, format_pan};
use iced::widget::text::Shaping;
use iced::widget::{
    button, canvas, column, container, mouse_area, opaque, row, slider, stack, text, Space,
};
use iced::{alignment, Color, Element, Font, Length};
use resonance_audio::types::*;

impl crate::Resonance {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let transport = self.view_transport();
        let main_area = match self.view_mode {
            ViewMode::Arrange => self.view_main_area(),
            ViewMode::Mixer => self.view_mixer(),
        };

        let content: Element<'_, Message> = if let Some(ref err) = self.error_message {
            let error_bar = container(
                row![
                    text(err).size(13).color(iced::Color::WHITE),
                    Space::with_width(Length::Fill),
                    button(text("\u{00d7}").size(14).color(iced::Color::WHITE))
                        .on_press(Message::DismissError)
                        .style(|_theme, _status| iced::widget::button::Style {
                            background: Some(iced::Background::Color(iced::Color::TRANSPARENT)),
                            text_color: iced::Color::WHITE,
                            ..Default::default()
                        })
                ]
                .spacing(8)
                .align_y(alignment::Vertical::Center)
                .padding(8),
            )
            .width(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::RECORD_RED)),
                ..Default::default()
            });
            column![transport, error_bar, main_area].spacing(0).into()
        } else {
            column![transport, main_area].spacing(0).into()
        };

        let base: Element<'_, Message> = container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::BG)),
                ..Default::default()
            })
            .into();

        if self.settings_open {
            stack![base, self.view_settings_overlay()].into()
        } else {
            base
        }
    }

    fn view_transport(&self) -> Element<'_, Message> {
        let tempo = TempoMap {
            bpm: self.bpm,
            numerator: self.time_sig_num,
            denominator: self.time_sig_den,
            metronome_enabled: self.metronome_enabled,
        };
        let bar_beat_str = tempo.format_position(self.playhead, self.sample_rate);

        let play_pause = if self.playing {
            button(text("\u{23f8}").size(18).color(theme::TEXT).shaping(Shaping::Advanced))
                .on_press(Message::Pause)
                .style(|_theme, status| theme::transport_button_style(status))
        } else {
            button(text("\u{25b6}").size(18).color(theme::ACCENT).shaping(Shaping::Advanced))
                .on_press(Message::Play)
                .style(|_theme, status| theme::transport_button_style(status))
        };

        let stop_btn = button(text("\u{23f9}").size(18).color(theme::TEXT).shaping(Shaping::Advanced))
            .on_press(Message::Stop)
            .style(|_theme, status| theme::transport_button_style(status));

        let skip_back = button(text("\u{23ea}").size(16).color(theme::TEXT).shaping(Shaping::Advanced))
            .on_press(Message::SkipBack)
            .style(|_theme, status| theme::transport_button_style(status));

        let skip_fwd = button(text("\u{23e9}").size(16).color(theme::TEXT).shaping(Shaping::Advanced))
            .on_press(Message::SkipForward)
            .style(|_theme, status| theme::transport_button_style(status));

        let time_display = text(bar_beat_str)
            .size(20)
            .font(Font::MONOSPACE)
            .color(theme::ACCENT);

        // BPM slider and display
        let bpm_slider = slider(20.0..=300.0, self.bpm, Message::SetBpm)
            .width(80)
            .step(1.0);
        let bpm_text = text(format!("{:.0}", self.bpm))
            .size(14)
            .font(Font::MONOSPACE)
            .color(theme::TEXT);
        let bpm_label = text("BPM").size(10).color(theme::TEXT_DIM);

        // Time signature button
        let time_sig_str = format!("{}/{}", self.time_sig_num, self.time_sig_den);
        let time_sig_btn = button(text(time_sig_str).size(14).font(Font::MONOSPACE).color(theme::TEXT))
            .on_press(Message::CycleTimeSignature)
            .style(|_theme, status| theme::transport_button_style(status));

        // Metronome toggle
        let met_color = if self.metronome_enabled {
            theme::METRONOME_ON
        } else {
            theme::TEXT_DIM
        };
        let metronome_enabled = self.metronome_enabled;
        let met_btn = button(text("Met").size(12).color(met_color))
            .on_press(Message::ToggleMetronome)
            .style(move |_theme, status| {
                theme::toggle_button_style(metronome_enabled, theme::METRONOME_ON, false, status)
            });

        // Recording indicator
        let rec_indicator = if self.recording {
            text("\u{25cf} REC").size(14).color(theme::RECORD_RED)
        } else {
            text("").size(14)
        };

        let zoom_out = button(text("\u{2212}").size(16).color(theme::TEXT))
            .on_press(Message::ZoomOut)
            .style(|_theme, status| theme::transport_button_style(status));

        let zoom_in = button(text("+").size(16).color(theme::TEXT))
            .on_press(Message::ZoomIn)
            .style(|_theme, status| theme::transport_button_style(status));

        let add_track = button(text("+ Track").size(14).color(theme::TEXT))
            .on_press(Message::AddTrack)
            .style(|_theme, status| theme::transport_button_style(status));

        let add_inst_track = button(text("+ Inst").size(14).color(Color::from_rgb(0.3, 0.75, 0.8)))
            .on_press(Message::AddInstrumentTrack)
            .style(|_theme, status| theme::transport_button_style(status));

        let open_btn = button(text("\u{1f4c2}").size(14).color(theme::TEXT).shaping(Shaping::Advanced))
            .on_press(Message::OpenProject)
            .style(|_theme, status| theme::transport_button_style(status));

        let save_btn = button(text("\u{1f4be}").size(14).color(theme::TEXT).shaping(Shaping::Advanced))
            .on_press(Message::SaveProject)
            .style(|_theme, status| theme::transport_button_style(status));

        let settings_btn = button(text("\u{2699}").size(16).color(theme::TEXT))
            .on_press(Message::OpenSettings)
            .style(|_theme, status| theme::transport_button_style(status));

        let punch_color = if self.punch_enabled {
            theme::PUNCH_MARKER
        } else {
            theme::TEXT_DIM
        };
        let punch_enabled = self.punch_enabled;
        let punch_btn = button(text("P").size(12).color(punch_color))
            .on_press(Message::TogglePunch)
            .style(move |_theme, status| {
                if punch_enabled {
                    let bg = match status {
                        iced::widget::button::Status::Hovered => {
                            iced::Color::from_rgb(0.25, 0.20, 0.10)
                        }
                        iced::widget::button::Status::Pressed => {
                            iced::Color::from_rgb(0.20, 0.15, 0.08)
                        }
                        _ => iced::Color::from_rgb(0.22, 0.18, 0.08),
                    };
                    iced::widget::button::Style {
                        background: Some(iced::Background::Color(bg)),
                        text_color: theme::PUNCH_MARKER,
                        border: iced::Border {
                            color: theme::PUNCH_MARKER,
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        ..Default::default()
                    }
                } else {
                    theme::transport_button_style(status)
                }
            });

        let master_slider = slider(-60.0..=6.0f32, self.master_volume, Message::SetMasterVolume)
            .width(80)
            .step(0.1);
        let master_vol_label = format_db(self.master_volume);

        let arrange_active = self.view_mode == ViewMode::Arrange;
        let mixer_active = self.view_mode == ViewMode::Mixer;
        let arrange_tab = button(text("Arrange").size(12))
            .on_press(Message::SwitchView(ViewMode::Arrange))
            .style(move |_theme, status| theme::tab_button_style(arrange_active, status))
            .padding([4, 8]);
        let mixer_tab = button(text("Mixer").size(12))
            .on_press(Message::SwitchView(ViewMode::Mixer))
            .style(move |_theme, status| theme::tab_button_style(mixer_active, status))
            .padding([4, 8]);

        let transport_row = row![
            Space::with_width(10),
            arrange_tab,
            mixer_tab,
            Space::with_width(8),
            skip_back,
            stop_btn,
            play_pause,
            skip_fwd,
            Space::with_width(16),
            time_display,
            Space::with_width(6),
            rec_indicator,
            Space::with_width(16),
            bpm_slider,
            bpm_text,
            bpm_label,
            Space::with_width(8),
            time_sig_btn,
            Space::with_width(4),
            met_btn,
            Space::with_width(4),
            punch_btn,
            Space::with_width(Length::Fill),
            text("Master").size(10).color(theme::TEXT_DIM),
            master_slider,
            text(master_vol_label).size(11).font(Font::MONOSPACE).color(theme::TEXT_DIM),
            Space::with_width(12),
            zoom_out,
            text("Zoom").size(12).color(theme::TEXT_DIM),
            zoom_in,
            Space::with_width(20),
            add_track,
            add_inst_track,
            Space::with_width(6),
            open_btn,
            save_btn,
            Space::with_width(6),
            settings_btn,
            Space::with_width(10),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center)
        .height(48);

        container(transport_row)
            .width(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::PANEL)),
                border: iced::Border {
                    color: theme::SEPARATOR,
                    width: 0.0,
                    radius: 0.0.into(),
                },
                ..Default::default()
            })
            .into()
    }

    fn view_settings_overlay(&self) -> Element<'_, Message> {
        let backdrop = mouse_area(
            container(Space::new(Length::Fill, Length::Fill))
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_theme| container::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgba(
                        0.0, 0.0, 0.0, 0.6,
                    ))),
                    ..Default::default()
                }),
        )
        .on_press(Message::CloseSettings);

        let title = text("Settings").size(20).color(theme::ACCENT);

        let close_btn = button(text("Close").size(14).color(theme::TEXT))
            .on_press(Message::CloseSettings)
            .style(|_theme, status| theme::transport_button_style(status));

        let dialog_content = column![
            title,
            Space::with_height(20),
            close_btn,
        ]
        .spacing(8)
        .padding(24)
        .width(360);

        let dialog = container(dialog_content).style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::PANEL)),
            border: iced::Border {
                color: theme::SEPARATOR,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        });

        let centered = container(opaque(dialog))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill);

        stack![backdrop, centered].into()
    }

    fn view_main_area(&self) -> Element<'_, Message> {
        let track_headers = self.view_track_headers();
        let timeline = self.view_timeline();

        let main = row![track_headers, timeline];

        if let Some(ref editor_state) = self.editing_midi_clip {
            // Find the clip being edited
            if let Some(clip) = self.midi_clips.iter().find(|c| c.id == editor_state.clip_id) {
                let close_btn = button(text("Close Editor").size(12).color(theme::TEXT))
                    .on_press(Message::CloseMidiEditor)
                    .style(|_theme, status| theme::transport_button_style(status))
                    .padding([4, 8]);
                let editor_label = text(format!("MIDI: {}", clip.name))
                    .size(12)
                    .color(theme::ACCENT);
                let editor_toolbar = container(
                    row![editor_label, Space::with_width(Length::Fill), close_btn]
                        .spacing(8)
                        .align_y(alignment::Vertical::Center)
                        .padding([4, 8]),
                )
                .width(Length::Fill)
                .style(|_theme| container::Style {
                    background: Some(iced::Background::Color(theme::PANEL)),
                    border: iced::Border {
                        color: theme::SEPARATOR,
                        width: 1.0,
                        radius: 0.0.into(),
                    },
                    ..Default::default()
                });

                let piano_roll = canvas(PianoRollCanvas {
                    clip,
                    track_id: editor_state.track_id,
                    scroll_x: editor_state.scroll_x,
                    scroll_y: editor_state.scroll_y,
                    zoom_x: editor_state.zoom_x,
                    zoom_y: editor_state.zoom_y,
                    snap_ticks: editor_state.snap_ticks,
                    selected_note: editor_state.selected_note,
                    time_sig_num: self.time_sig_num,
                })
                .width(Length::Fill)
                .height(Length::Fill);

                let editor_panel = column![editor_toolbar, piano_roll].spacing(0);

                let editor_container = container(editor_panel)
                    .width(Length::Fill)
                    .height(250)
                    .style(|_theme| container::Style {
                        background: Some(iced::Background::Color(theme::BG)),
                        border: iced::Border {
                            color: theme::SEPARATOR,
                            width: 1.0,
                            radius: 0.0.into(),
                        },
                        ..Default::default()
                    });

                return column![
                    container(main).width(Length::Fill).height(Length::Fill),
                    editor_container,
                ]
                .spacing(0)
                .into();
            }
        }

        container(main)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_track_headers(&self) -> Element<'_, Message> {
        let mut headers = column![].spacing(0);

        // Ruler header spacer
        headers = headers.push(
            container(Space::new(Length::Fill, theme::RULER_HEIGHT)).style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::PANEL_DARK)),
                ..Default::default()
            }),
        );

        let sorted_tracks = self.sorted_tracks();

        // Calculate which tracks are visible given scroll_offset_y
        let visible_start = self.scroll_offset_y / theme::TRACK_HEIGHT;
        let first_visible = visible_start.floor() as usize;
        // Add top padding for the scrolled-away portion
        let top_pad = first_visible as f32 * theme::TRACK_HEIGHT - self.scroll_offset_y;
        if first_visible > 0 {
            headers = headers.push(Space::new(Length::Fill, (first_visible as f32 * theme::TRACK_HEIGHT - self.scroll_offset_y).max(0.0)));
        } else if self.scroll_offset_y > 0.0 {
            // Partial first track: use negative-ish padding -- just skip offset
            headers = headers.push(Space::new(Length::Fill, top_pad.max(0.0)));
        }

        for (i, track) in sorted_tracks.iter().enumerate() {
            if i < first_visible {
                continue;
            }
            let header = self.view_track_header(track);
            headers = headers.push(header);
        }

        container(headers)
            .width(theme::TRACK_HEADER_WIDTH)
            .height(Length::Fill)
            .clip(true)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::PANEL)),
                border: iced::Border {
                    color: theme::SEPARATOR,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..Default::default()
            })
            .into()
    }

    fn view_track_header(&self, track: &TrackState) -> Element<'_, Message> {
        let name = text(track.name.clone()).size(13).color(theme::TEXT);

        // Record arm button
        let rec_color = if track.record_armed {
            theme::RECORD_RED
        } else {
            theme::TEXT_DIM
        };
        let armed = track.record_armed;
        let rec_btn = button(text("R").size(11).color(rec_color))
            .on_press(Message::ToggleRecordArm(track.id))
            .style(move |_theme, status| {
                if armed {
                    theme::record_armed_button_style(status)
                } else {
                    theme::small_button_style(status)
                }
            })
            .padding(2);

        let mute_color = if track.muted {
            theme::ACCENT
        } else {
            theme::TEXT_DIM
        };
        let mute_btn = button(text("M").size(11).color(mute_color))
            .on_press(Message::ToggleMute(track.id))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2);

        let solo_color = if track.soloed {
            theme::SOLO_YELLOW
        } else {
            theme::TEXT_DIM
        };
        let solo_btn = button(text("S").size(11).color(solo_color))
            .on_press(Message::ToggleSolo(track.id))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2);

        let vol_slider = slider(-60.0..=6.0f32, track.volume, {
            let id = track.id;
            move |v| Message::SetTrackVolume(id, v)
        })
        .width(80)
        .step(0.1);

        let vol_label = format_db(track.volume);
        let vol_text = text(vol_label)
            .size(11)
            .font(Font::MONOSPACE)
            .color(theme::TEXT_DIM);

        let pan_slider = slider(-1.0..=1.0f32, track.pan, {
            let id = track.id;
            move |v| Message::SetTrackPan(id, v)
        })
        .width(50)
        .step(0.01);

        let pan_label = format_pan(track.pan);
        let pan_text = text(pan_label)
            .size(11)
            .font(Font::MONOSPACE)
            .color(theme::TEXT_DIM);

        let import_btn: Element<'_, Message> = if track.track_type == TrackType::Instrument {
            button(text("+M").size(11).color(Color::from_rgb(0.3, 0.75, 0.8)))
                .on_press(Message::CreateMidiClip(track.id))
                .style(|_theme, status| theme::small_button_style(status))
                .padding(2)
                .into()
        } else {
            button(text("+").size(12).color(theme::TEXT))
                .on_press(Message::ImportFile(track.id))
                .style(|_theme, status| theme::small_button_style(status))
                .padding(2)
                .into()
        };

        let del_btn = button(text("\u{00d7}").size(12).color(theme::TEXT_DIM))
            .on_press(Message::RemoveTrack(track.id))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2);

        // Monitor button
        let mon_color = if track.monitor_enabled {
            theme::METRONOME_ON
        } else {
            theme::TEXT_DIM
        };
        let mon_enabled = track.monitor_enabled;
        let mon_btn = button(text("I").size(11).color(mon_color))
            .on_press(Message::ToggleMonitor(track.id))
            .style(move |_theme, status| {
                theme::toggle_button_style(mon_enabled, theme::METRONOME_ON, true, status)
            })
            .padding(2);

        // Mono/Stereo toggle
        let mono_label = if track.mono { "M" } else { "S" };
        let is_mono = track.mono;
        let mono_btn = button(text(mono_label).size(11).color(theme::TEXT))
            .on_press(Message::ToggleTrackMono(track.id))
            .style(move |_theme, status| {
                theme::mono_button_style(is_mono, status)
            })
            .padding(2);

        let top_row = row![
            name,
            Space::with_width(Length::Fill),
            mono_btn,
            mon_btn,
            rec_btn,
            mute_btn,
            solo_btn,
            import_btn,
            del_btn
        ]
        .spacing(4)
        .align_y(alignment::Vertical::Center);

        let bottom_row = row![vol_slider, vol_text, pan_slider, pan_text]
            .spacing(4)
            .align_y(alignment::Vertical::Center);

        let content = column![top_row, bottom_row].spacing(2).padding(6);

        let bg = if track.record_armed {
            theme::PANEL_ARMED
        } else {
            theme::PANEL_DARK
        };
        let border_color = if track.record_armed {
            theme::RECORD_RED
        } else {
            theme::SEPARATOR
        };

        container(content)
            .width(Length::Fill)
            .height(theme::TRACK_HEIGHT)
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

    fn view_timeline(&self) -> Element<'_, Message> {
        let recording_tracks: Vec<TrackId> = if self.recording {
            self.tracks
                .iter()
                .filter(|t| t.record_armed)
                .map(|t| t.id)
                .collect()
        } else {
            Vec::new()
        };

        let timeline_data = TimelineCanvas {
            tracks: &self.tracks,
            clips: &self.clips,
            playhead: self.playhead,
            sample_rate: self.sample_rate,
            zoom: self.zoom,
            scroll_offset: self.scroll_offset,
            recording_tracks,
            recording_start_sample: self.recording_start_sample,
            bpm: self.bpm,
            time_sig_num: self.time_sig_num,
            scroll_offset_y: self.scroll_offset_y,
            punch_enabled: self.punch_enabled,
            punch_in: self.punch_in,
            punch_out: self.punch_out,
            selected_clip: self.selected_clip,
            midi_clips: &self.midi_clips,
            selected_midi_clip: self.selected_midi_clip,
        };

        canvas(timeline_data)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
