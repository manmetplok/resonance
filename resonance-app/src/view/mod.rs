/// View rendering for the Resonance application.
pub(crate) mod compose;
pub(crate) mod mixer;

use crate::message::Message;
use crate::midi_editor::PianoRollCanvas;
use crate::state::*;
use crate::theme;
use crate::theme::fa;
use crate::timeline::TimelineCanvas;
use iced::widget::text::LineHeight;
use iced::widget::{
    button, canvas, column, container, mouse_area, opaque, row, stack, text, text_input, Space,
};
use iced::{alignment, Color, Element, Font, Length};
use resonance_audio::types::*;

impl crate::Resonance {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let transport = self.view_transport();
        let main_area = match self.view_mode {
            ViewMode::Arrange => self.view_main_area(),
            ViewMode::Mixer => self.view_mixer(),
            ViewMode::Compose => self.view_compose(),
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
        } else if self.add_track_menu_open {
            stack![base, self.view_add_track_menu()].into()
        } else {
            base
        }
    }

    fn view_add_track_menu(&self) -> Element<'_, Message> {
        let backdrop = mouse_area(
            container(Space::new(Length::Fill, Length::Fill))
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_theme| container::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgba(
                        0.0, 0.0, 0.0, 0.3,
                    ))),
                    ..Default::default()
                }),
        )
        .on_press(Message::CloseAddTrackMenu);

        let audio_btn = button(
            row![
                theme::icon(theme::fa::MICROPHONE).size(14).color(theme::TEXT),
                Space::with_width(8),
                text("Audio").size(13).color(theme::TEXT),
            ]
            .align_y(alignment::Vertical::Center),
        )
        .on_press(Message::AddTrack)
        .width(Length::Fill)
        .padding([6, 10])
        .style(|_theme, status| theme::transport_button_style(status));

        let inst_btn = button(
            row![
                theme::icon(theme::fa::MUSIC).size(14).color(Color::from_rgb(0.3, 0.75, 0.8)),
                Space::with_width(8),
                text("Instrument").size(13).color(Color::from_rgb(0.3, 0.75, 0.8)),
            ]
            .align_y(alignment::Vertical::Center),
        )
        .on_press(Message::AddInstrumentTrack)
        .width(Length::Fill)
        .padding([6, 10])
        .style(|_theme, status| theme::transport_button_style(status));

        let menu_content = column![
            text("Add Track").size(11).color(theme::TEXT_DIM),
            Space::with_height(4),
            audio_btn,
            inst_btn,
        ]
        .spacing(2)
        .padding(8)
        .width(180);

        let menu = container(opaque(menu_content)).style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::PANEL)),
            border: iced::Border {
                color: theme::SEPARATOR,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..Default::default()
        });

        // Position the popup just below the "+" button in the track header ruler area.
        // The + button sits near the left edge of the track header column (~12px in)
        // directly below the transport bar (transport height ~48px + ruler height 30px).
        let top_pad = 48.0 + theme::RULER_HEIGHT + 2.0;
        let positioned = container(menu)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(alignment::Horizontal::Left)
            .align_y(alignment::Vertical::Top)
            .padding(iced::Padding {
                top: top_pad,
                right: 0.0,
                bottom: 0.0,
                left: 12.0,
            });

        stack![backdrop, positioned].into()
    }

    fn view_transport(&self) -> Element<'_, Message> {
        let tempo = TempoMap {
            bpm: self.bpm,
            numerator: self.time_sig_num,
            denominator: self.time_sig_den,
            metronome_enabled: self.metronome_enabled,
        };
        let bar_beat_str = tempo.format_position(self.playhead, self.sample_rate);
        let time_str = tempo.format_time(self.playhead, self.sample_rate);

        // ---- Transport buttons (uniform size/padding/style) -----------------
        const TRANSPORT_ICON_SIZE: u16 = 16;
        let button_pad = iced::Padding::from([6, 10]);

        let skip_back = button(
            theme::icon(fa::BACKWARD_STEP)
                .size(TRANSPORT_ICON_SIZE)
                .color(theme::TEXT),
        )
        .on_press(Message::SkipBack)
        .padding(button_pad)
        .style(|_theme, status| theme::transport_button_style(status));

        let stop_btn = button(
            theme::icon(fa::STOP)
                .size(TRANSPORT_ICON_SIZE)
                .color(theme::TEXT),
        )
        .on_press(Message::Stop)
        .padding(button_pad)
        .style(|_theme, status| theme::transport_button_style(status));

        let play_pause: Element<'_, Message> = if self.playing {
            button(
                theme::icon(fa::PAUSE)
                    .size(TRANSPORT_ICON_SIZE)
                    .color(theme::TEXT),
            )
            .on_press(Message::Pause)
            .padding(button_pad)
            .style(|_theme, status| theme::transport_button_style(status))
            .into()
        } else {
            button(
                theme::icon(fa::PLAY)
                    .size(TRANSPORT_ICON_SIZE)
                    .color(theme::ACCENT),
            )
            .on_press(Message::Play)
            .padding(button_pad)
            .style(|_theme, status| theme::transport_button_style(status))
            .into()
        };

        let skip_fwd = button(
            theme::icon(fa::FORWARD_STEP)
                .size(TRANSPORT_ICON_SIZE)
                .color(theme::TEXT),
        )
        .on_press(Message::SkipForward)
        .padding(button_pad)
        .style(|_theme, status| theme::transport_button_style(status));

        // Record button: grayed out and unclickable when no track is armed.
        let any_armed = self.tracks.iter().any(|t| t.record_armed);
        let rec_color = if any_armed {
            theme::RECORD_RED
        } else {
            theme::TEXT_DIM
        };
        let mut rec_btn = button(
            theme::icon(fa::CIRCLE)
                .size(TRANSPORT_ICON_SIZE)
                .color(rec_color),
        )
        .padding(button_pad)
        .style(move |_theme, status| {
            if any_armed {
                theme::record_armed_button_style(status)
            } else {
                theme::transport_button_style(status)
            }
        });
        if any_armed {
            rec_btn = rec_btn.on_press(Message::Record);
        }

        // ---- Fancy timing panel ---------------------------------------------
        //
        // Every sub-block is a two-row column with identical structure so
        // all values share the same baseline. Every text element uses
        // `line_height(1.0)` so its layout box equals its font-size — this
        // is critical because the icon font and monospace font have wildly
        // different hhea line metrics, and centering within a fixed-height
        // row otherwise pushes them to different vertical positions.
        //
        //   row 1 (VALUE_ROW_HEIGHT): big value, size 18
        //   row 2 (LABEL_ROW_HEIGHT): small label, size 9

        const VALUE_SIZE: u16 = 18;
        const LABEL_SIZE: u16 = 9;
        const BLOCK_HEIGHT: f32 = 40.0;
        const VALUE_ROW_HEIGHT: f32 = 22.0;
        const LABEL_ROW_HEIGHT: f32 = 12.0;

        let tight = LineHeight::Relative(1.0);

        // Wrap any value-row content in a fixed-height centered cell.
        fn value_cell<'a>(
            content: impl Into<Element<'a, Message>>,
        ) -> iced::widget::Container<'a, Message> {
            container(content)
                .width(Length::Fill)
                .height(VALUE_ROW_HEIGHT)
                .align_x(alignment::Horizontal::Center)
                .align_y(alignment::Vertical::Center)
        }

        fn label_cell<'a>(
            content: impl Into<Element<'a, Message>>,
        ) -> iced::widget::Container<'a, Message> {
            container(content)
                .width(Length::Fill)
                .height(LABEL_ROW_HEIGHT)
                .align_x(alignment::Horizontal::Center)
                .align_y(alignment::Vertical::Center)
        }

        // Position block: bars.beats value, mm:ss.xxx "label" (dim time).
        let position_block = column![
            value_cell(
                text(bar_beat_str)
                    .size(VALUE_SIZE)
                    .line_height(tight)
                    .font(Font::MONOSPACE)
                    .color(theme::ACCENT),
            ),
            label_cell(
                text(time_str)
                    .size(LABEL_SIZE + 1)
                    .line_height(tight)
                    .font(Font::MONOSPACE)
                    .color(theme::TEXT_DIM),
            ),
        ]
        .width(112)
        .align_x(alignment::Horizontal::Center);

        // BPM block: editable number with "BPM" label.
        let bpm_field = text_input("120", &self.bpm_input)
            .on_input(Message::SetBpmText)
            .on_submit(Message::CommitBpm)
            .width(52)
            .size(VALUE_SIZE)
            .font(Font::MONOSPACE)
            .align_x(alignment::Horizontal::Center)
            .padding(0)
            .style(theme::borderless_text_input_style);
        let bpm_block = column![
            value_cell(bpm_field),
            label_cell(
                text("BPM")
                    .size(LABEL_SIZE)
                    .line_height(tight)
                    .color(theme::TEXT_DIM),
            ),
        ]
        .width(60)
        .align_x(alignment::Horizontal::Center);

        // Time signature block: clickable value, "SIG" label.
        let time_sig_str = format!("{}/{}", self.time_sig_num, self.time_sig_den);
        let time_sig_value = mouse_area(
            text(time_sig_str)
                .size(VALUE_SIZE)
                .line_height(tight)
                .font(Font::MONOSPACE)
                .color(theme::TEXT),
        )
        .on_press(Message::CycleTimeSignature);
        let time_sig_block = column![
            value_cell(time_sig_value),
            label_cell(
                text("SIG")
                    .size(LABEL_SIZE)
                    .line_height(tight)
                    .color(theme::TEXT_DIM),
            ),
        ]
        .width(48)
        .align_x(alignment::Horizontal::Center);

        // Metronome block: clickable icon on top, precount bars setting below.
        let met_color = if self.metronome_enabled {
            theme::METRONOME_ON
        } else {
            theme::TEXT_DIM
        };
        let met_icon = mouse_area(
            theme::icon(fa::METRONOME)
                .size(VALUE_SIZE)
                .line_height(tight)
                .color(met_color),
        )
        .on_press(Message::ToggleMetronome);

        let precount_label = if self.precount_bars == 0 {
            "OFF".to_string()
        } else {
            format!("{} BAR", self.precount_bars)
        };
        let precount_text = mouse_area(
            text(precount_label)
                .size(LABEL_SIZE)
                .line_height(tight)
                .font(Font::MONOSPACE)
                .color(theme::TEXT_DIM),
        )
        .on_press(Message::CyclePrecountBars);

        let met_block = column![
            value_cell(met_icon),
            label_cell(precount_text),
        ]
        .width(52)
        .align_x(alignment::Horizontal::Center);

        // Thin vertical separator between sub-blocks.
        let sep = || {
            container(Space::new(1, BLOCK_HEIGHT - 12.0))
                .style(|_theme| container::Style {
                    background: Some(iced::Background::Color(theme::SEPARATOR)),
                    ..Default::default()
                })
        };

        let timing_panel_row = row![
            position_block,
            sep(),
            bpm_block,
            sep(),
            time_sig_block,
            sep(),
            met_block,
        ]
        .spacing(10)
        .align_y(alignment::Vertical::Center)
        .height(BLOCK_HEIGHT);

        let timing_panel = container(timing_panel_row)
            .padding(iced::Padding::from([4, 12]))
            .style(theme::timing_panel_style);

        // ---- Punch toggle (icon button in transport area) ------------------
        let punch_color = if self.punch_enabled {
            theme::PUNCH_MARKER
        } else {
            theme::TEXT_DIM
        };
        let punch_enabled = self.punch_enabled;
        let punch_btn = button(
            theme::icon(fa::BULLSEYE)
                .size(TRANSPORT_ICON_SIZE)
                .color(punch_color),
        )
        .on_press(Message::TogglePunch)
        .padding(button_pad)
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

        // ---- Settings icon (Font Awesome bars) ------------------------------
        let settings_btn = button(
            theme::icon(fa::BARS)
                .size(TRANSPORT_ICON_SIZE)
                .color(theme::TEXT),
        )
        .on_press(Message::OpenSettings)
        .padding(button_pad)
        .style(|_theme, status| theme::transport_button_style(status));

        // ---- View mode tabs -------------------------------------------------
        let arrange_active = self.view_mode == ViewMode::Arrange;
        let mixer_active = self.view_mode == ViewMode::Mixer;
        let compose_active = self.view_mode == ViewMode::Compose;
        let arrange_tab = button(text("Arrange").size(12))
            .on_press(Message::SwitchView(ViewMode::Arrange))
            .style(move |_theme, status| theme::tab_button_style(arrange_active, status))
            .padding([4, 8]);
        let mixer_tab = button(text("Mixer").size(12))
            .on_press(Message::SwitchView(ViewMode::Mixer))
            .style(move |_theme, status| theme::tab_button_style(mixer_active, status))
            .padding([4, 8]);
        let compose_tab = button(text("Compose").size(12))
            .on_press(Message::SwitchView(ViewMode::Compose))
            .style(move |_theme, status| theme::tab_button_style(compose_active, status))
            .padding([4, 8]);

        // ---- Final row assembly ---------------------------------------------
        let transport_row = row![
            Space::with_width(10),
            arrange_tab,
            mixer_tab,
            compose_tab,
            Space::with_width(10),
            skip_back,
            stop_btn,
            play_pause,
            rec_btn,
            skip_fwd,
            Space::with_width(8),
            punch_btn,
            Space::with_width(16),
            timing_panel,
            Space::with_width(Length::Fill),
            settings_btn,
            Space::with_width(10),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center)
        .height(56);

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

        let section = |label: &'static str| {
            text(label).size(11).color(theme::TEXT_DIM)
        };

        let open_btn = button(
            row![
                theme::icon(fa::FOLDER_OPEN).size(14).color(theme::TEXT),
                Space::with_width(10),
                text("Open Project...").size(13).color(theme::TEXT),
            ]
            .align_y(alignment::Vertical::Center),
        )
        .on_press(Message::OpenProject)
        .padding([8, 14])
        .width(Length::Fill)
        .style(|_theme, status| theme::transport_button_style(status));

        let save_btn = button(
            row![
                theme::icon(fa::FLOPPY_DISK).size(14).color(theme::TEXT),
                Space::with_width(10),
                text("Save Project").size(13).color(theme::TEXT),
            ]
            .align_y(alignment::Vertical::Center),
        )
        .on_press(Message::SaveProject)
        .padding([8, 14])
        .width(Length::Fill)
        .style(|_theme, status| theme::transport_button_style(status));

        let save_as_btn = button(
            row![
                theme::icon(fa::FLOPPY_DISK).size(14).color(theme::TEXT),
                Space::with_width(10),
                text("Save Project As...").size(13).color(theme::TEXT),
            ]
            .align_y(alignment::Vertical::Center),
        )
        .on_press(Message::SaveProjectAs)
        .padding([8, 14])
        .width(Length::Fill)
        .style(|_theme, status| theme::transport_button_style(status));

        let close_btn = button(text("Close").size(13).color(theme::TEXT))
            .on_press(Message::CloseSettings)
            .padding([6, 14])
            .style(|_theme, status| theme::transport_button_style(status));

        let dialog_content = column![
            title,
            Space::with_height(16),
            section("Project"),
            Space::with_height(6),
            open_btn,
            save_btn,
            save_as_btn,
            Space::with_height(20),
            row![
                Space::with_width(Length::Fill),
                close_btn,
            ],
        ]
        .spacing(6)
        .padding(24)
        .width(420);

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

        if let Some(editor) = self.view_midi_editor_panel() {
            return column![
                container(main).width(Length::Fill).height(Length::Fill),
                editor,
            ]
            .spacing(0)
            .into();
        }

        container(main)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// Bottom MIDI editor panel shown whenever a clip is open in the piano
    /// roll. Used by both the Arrange and Compose tabs so inline editing
    /// works identically from either view.
    pub(crate) fn view_midi_editor_panel(&self) -> Option<Element<'_, Message>> {
        let editor_state = self.editing_midi_clip.as_ref()?;
        let clip = self
            .midi_clips
            .iter()
            .find(|c| c.id == editor_state.clip_id)?;

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

        Some(
            container(editor_panel)
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
                })
                .into(),
        )
    }

    pub(crate) fn view_track_headers(&self) -> Element<'_, Message> {
        let mut headers = column![].spacing(0);

        // Ruler header with "+" button to add a track
        let add_btn = button(text("+").size(16).color(theme::TEXT))
            .on_press(Message::OpenAddTrackMenu)
            .style(|_theme, status| theme::small_button_style(status))
            .padding([0, 6]);
        let add_row = row![Space::with_width(6), add_btn]
            .align_y(alignment::Vertical::Center)
            .height(theme::RULER_HEIGHT);
        headers = headers.push(
            container(add_row)
                .width(Length::Fill)
                .height(theme::RULER_HEIGHT)
                .style(|_theme| container::Style {
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
        let is_sub = track.sub_track.is_some();
        // Track name on its own line, clipped at the header width so long
        // names don't push the icons offscreen. `Wrapping::None` prevents
        // iced from line-wrapping and the enclosing container's clip flag
        // trims any glyph that overflows the available width. Sub-tracks
        // render dimmer since they're driven by their parent plugin.
        let name_color = if is_sub { theme::TEXT_DIM } else { theme::TEXT };
        let name = text(track.name.clone())
            .size(13)
            .color(name_color)
            .wrapping(iced::widget::text::Wrapping::None);
        let name_row = container(name)
            .width(Length::Fill)
            .clip(true);

        // Record arm (filled circle; red when armed).
        let rec_color = if track.record_armed {
            theme::RECORD_RED
        } else {
            theme::TEXT_DIM
        };
        let armed = track.record_armed;
        let rec_btn = button(
            theme::icon(theme::fa::CIRCLE).size(12).color(rec_color),
        )
        .on_press(Message::ToggleRecordArm(track.id))
        .style(move |_theme, status| {
            if armed {
                theme::record_armed_button_style(status)
            } else {
                theme::small_button_style(status)
            }
        })
        .padding(2);

        // Mute (speaker with X; accent when muted).
        let mute_color = if track.muted {
            theme::ACCENT
        } else {
            theme::TEXT_DIM
        };
        let mute_btn = button(
            theme::icon(theme::fa::VOLUME_XMARK).size(12).color(mute_color),
        )
        .on_press(Message::ToggleMute(track.id))
        .style(|_theme, status| theme::small_button_style(status))
        .padding(2);

        // Solo (headphones; yellow when soloed).
        let solo_color = if track.soloed {
            theme::SOLO_YELLOW
        } else {
            theme::TEXT_DIM
        };
        let solo_btn = button(
            theme::icon(theme::fa::HEADPHONES).size(12).color(solo_color),
        )
        .on_press(Message::ToggleSolo(track.id))
        .style(|_theme, status| theme::small_button_style(status))
        .padding(2);

        let del_btn = button(
            theme::icon(theme::fa::TRASH).size(12).color(theme::TEXT_DIM),
        )
        .on_press(Message::RemoveTrack(track.id))
        .style(|_theme, status| theme::small_button_style(status))
        .padding(2);

        // Monitor (eye; green when monitoring).
        let mon_color = if track.monitor_enabled {
            theme::METRONOME_ON
        } else {
            theme::TEXT_DIM
        };
        let mon_enabled = track.monitor_enabled;
        let mon_btn = button(
            theme::icon(theme::fa::EYE).size(12).color(mon_color),
        )
        .on_press(Message::ToggleMonitor(track.id))
        .style(move |_theme, status| {
            theme::toggle_button_style(mon_enabled, theme::METRONOME_ON, true, status)
        })
        .padding(2);

        // Mono/Stereo toggle: one hollow circle for mono, two overlapping
        // hollow circles for stereo (the classic stereo symbol —
        // https://de.wikipedia.org/wiki/Datei:Stereo2.png). Both glyphs
        // are custom additions to our extended FA font — see
        // tools/add_mono_stereo_glyphs.py.
        let is_mono = track.mono;
        let mono_glyph = if is_mono {
            theme::fa::CIRCLE_HOLLOW
        } else {
            theme::fa::CIRCLE_HOLLOW_DOUBLE
        };
        let mono_btn = button(
            theme::icon(mono_glyph).size(12).color(theme::TEXT),
        )
        .on_press(Message::ToggleTrackMono(track.id))
        .style(move |_theme, status| theme::mono_button_style(is_mono, status))
        .padding(2);

        // Sub-tracks expose a trimmed toolbar: just mute/solo + a
        // per-port label. They cannot be armed, monitored, deleted, or
        // swapped to mono — those all belong to their parent.
        let icon_row: iced::widget::Row<'_, Message> = if is_sub {
            row![
                mute_btn,
                solo_btn,
                Space::with_width(Length::Fill),
            ]
            .spacing(4)
            .align_y(alignment::Vertical::Center)
        } else {
            row![
                mono_btn,
                mon_btn,
                rec_btn,
                mute_btn,
                solo_btn,
                del_btn,
                Space::with_width(Length::Fill),
            ]
            .spacing(4)
            .align_y(alignment::Vertical::Center)
        };

        let header_col = column![name_row, icon_row].spacing(4);

        // Sub-tracks get an indent on the left so their visual hierarchy
        // under the parent track is obvious at a glance. iced Padding's
        // array form is [vertical, horizontal] only, so we build the full
        // Padding explicitly for the per-side indent.
        let left_pad: f32 = if is_sub { 20.0 } else { 8.0 };
        let content = container(header_col)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding {
                top: 6.0,
                right: 8.0,
                bottom: 6.0,
                left: left_pad,
            })
            .clip(true);

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

        let canvas_el = canvas(timeline_data)
            .width(Length::Fill)
            .height(Length::Fill);

        // Floating zoom buttons, anchored to the bottom-right corner of the
        // timeline. Using Length::Shrink so the overlay only hit-tests the
        // buttons themselves — clicks elsewhere pass through to the canvas.
        let zoom_out = button(
            theme::icon(fa::MAGNIFYING_GLASS_MINUS)
                .size(12)
                .color(theme::TEXT),
        )
        .on_press(Message::ZoomOut)
        .padding([6, 8])
        .style(|_theme, status| theme::floating_button_style(status));

        let zoom_in = button(
            theme::icon(fa::MAGNIFYING_GLASS_PLUS)
                .size(12)
                .color(theme::TEXT),
        )
        .on_press(Message::ZoomIn)
        .padding([6, 8])
        .style(|_theme, status| theme::floating_button_style(status));

        let zoom_group = row![zoom_out, zoom_in].spacing(4);

        // Position the button cluster in the bottom-right of the canvas.
        // Reserve some bottom padding so it clears the horizontal scrollbar
        // strip drawn inside the canvas (10 px) plus breathing room.
        let overlay = container(
            column![
                Space::with_height(Length::Fill),
                row![Space::with_width(Length::Fill), zoom_group],
            ]
            .spacing(0),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(iced::Padding {
            top: 0.0,
            right: 20.0,
            bottom: 20.0,
            left: 0.0,
        });

        stack![canvas_el, overlay].into()
    }
}
