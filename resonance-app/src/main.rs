use iced::widget::{button, canvas, column, container, pick_list, row, scrollable, slider, text, Space};
use iced::{alignment, Element, Font, Length, Size, Subscription};
use resonance_audio::types::*;
use resonance_audio::AudioEngine;

mod theme;
mod timeline;

use timeline::TimelineCanvas;

/// Application state.
struct Resonance {
    engine: AudioEngine,
    tracks: Vec<TrackState>,
    clips: Vec<ClipState>,
    playhead: u64,
    playing: bool,
    recording: bool,
    recording_start_sample: u64,
    sample_rate: u32,
    zoom: f32,          // pixels per second
    scroll_offset: f32, // horizontal scroll in pixels
    next_track_order: usize,
    input_devices: Vec<InputDeviceInfo>,
    default_input_device_name: Option<String>,
    bpm: f32,
    time_sig_num: u8,
    time_sig_den: u8,
    metronome_enabled: bool,
}

/// GUI-side track state.
#[derive(Debug, Clone)]
pub struct TrackState {
    pub id: TrackId,
    pub name: String,
    pub volume: f32,
    pub muted: bool,
    pub order: usize,
    pub record_armed: bool,
    pub input_device_name: Option<String>,
}

/// GUI-side clip state.
#[derive(Debug, Clone)]
pub struct ClipState {
    pub id: ClipId,
    pub track_id: TrackId,
    pub start_sample: SamplePos,
    pub duration_samples: u64,
    pub name: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum Message {
    Play,
    Pause,
    Stop,
    SkipBack,
    SkipForward,
    AddTrack,
    RemoveTrack(TrackId),
    SetTrackVolume(TrackId, f32),
    ToggleMute(TrackId),
    ImportFile(TrackId),
    FileSelected(TrackId, Option<String>),
    DeleteClip(ClipId),
    ZoomIn,
    ZoomOut,
    Tick,
    ToggleRecordArm(TrackId),
    SetTrackInputDevice(TrackId, Option<String>),
    SetBpm(f32),
    ToggleMetronome,
    CycleTimeSignature,
}

fn main() -> iced::Result {
    iced::application("Resonance", Resonance::update, Resonance::view)
        .subscription(Resonance::subscription)
        .theme(|_| theme::resonance_theme())
        .window_size(Size::new(1280.0, 720.0))
        .run_with(Resonance::new)
}

impl Resonance {
    fn new() -> (Self, iced::Task<Message>) {
        let engine = AudioEngine::new().expect("Failed to initialize audio engine");

        // Request input device list on startup
        engine.send(AudioCommand::ListInputDevices);

        let app = Self {
            engine,
            tracks: Vec::new(),
            clips: Vec::new(),
            playhead: 0,
            playing: false,
            recording: false,
            recording_start_sample: 0,
            sample_rate: 44100, // overwritten by SampleRateDetected event
            zoom: 100.0,
            scroll_offset: 0.0,
            next_track_order: 0,
            input_devices: Vec::new(),
            default_input_device_name: None,
            bpm: 120.0,
            time_sig_num: 4,
            time_sig_den: 4,
            metronome_enabled: false,
        };

        (app, iced::Task::none())
    }

    fn update(&mut self, message: Message) -> iced::Task<Message> {
        match message {
            Message::Play => {
                self.engine.send(AudioCommand::Play);
                self.playing = true;
            }
            Message::Pause => {
                self.engine.send(AudioCommand::Pause);
                self.playing = false;
            }
            Message::Stop => {
                self.engine.send(AudioCommand::Stop);
                self.playing = false;
                self.playhead = 0;
            }
            Message::SkipBack => {
                let skip = self.sample_rate as u64 * 5;
                let new_pos = self.playhead.saturating_sub(skip);
                self.engine.send(AudioCommand::SeekTo(new_pos));
                self.playhead = new_pos;
            }
            Message::SkipForward => {
                let skip = self.sample_rate as u64 * 5;
                let new_pos = self.playhead + skip;
                self.engine.send(AudioCommand::SeekTo(new_pos));
                self.playhead = new_pos;
            }
            Message::AddTrack => {
                self.engine.send(AudioCommand::AddTrack);
            }
            Message::RemoveTrack(id) => {
                self.engine.send(AudioCommand::RemoveTrack { track_id: id });
            }
            Message::SetTrackVolume(id, vol) => {
                self.engine.send(AudioCommand::SetTrackVolume {
                    track_id: id,
                    volume: vol,
                });
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == id) {
                    track.volume = vol;
                }
            }
            Message::ToggleMute(id) => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == id) {
                    track.muted = !track.muted;
                    self.engine.send(AudioCommand::SetTrackMute {
                        track_id: id,
                        muted: track.muted,
                    });
                }
            }
            Message::ImportFile(track_id) => {
                return iced::Task::perform(
                    async move {
                        let result = rfd::AsyncFileDialog::new()
                            .add_filter("Audio", &["wav", "mp3", "flac", "ogg"])
                            .set_title("Import Audio File")
                            .pick_file()
                            .await;
                        let path = result.map(|f| f.path().to_string_lossy().to_string());
                        (track_id, path)
                    },
                    move |(tid, path)| Message::FileSelected(tid, path),
                );
            }
            Message::FileSelected(track_id, Some(path)) => {
                self.engine.send(AudioCommand::ImportClip {
                    track_id,
                    path,
                    start_sample: self.playhead,
                });
            }
            Message::FileSelected(_, None) => {}
            Message::DeleteClip(id) => {
                self.engine.send(AudioCommand::DeleteClip { clip_id: id });
            }
            Message::ZoomIn => {
                self.zoom = (self.zoom * 1.5).min(1000.0);
            }
            Message::ZoomOut => {
                self.zoom = (self.zoom / 1.5).max(10.0);
            }
            Message::ToggleRecordArm(id) => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == id) {
                    track.record_armed = !track.record_armed;
                    // Auto-select default input device when arming if none set
                    if track.record_armed && track.input_device_name.is_none() {
                        if let Some(default) = &self.default_input_device_name {
                            track.input_device_name = Some(default.clone());
                            self.engine.send(AudioCommand::SetTrackInputDevice {
                                track_id: id,
                                device_name: Some(default.clone()),
                            });
                        }
                    }
                    self.engine.send(AudioCommand::SetTrackRecordArm {
                        track_id: id,
                        armed: track.record_armed,
                    });
                }
            }
            Message::SetTrackInputDevice(id, device_name) => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == id) {
                    track.input_device_name = device_name.clone();
                    self.engine.send(AudioCommand::SetTrackInputDevice {
                        track_id: id,
                        device_name,
                    });
                }
            }
            Message::SetBpm(bpm) => {
                self.bpm = bpm.clamp(20.0, 999.0);
                self.engine.send(AudioCommand::SetBpm { bpm: self.bpm });
            }
            Message::ToggleMetronome => {
                self.metronome_enabled = !self.metronome_enabled;
                self.engine.send(AudioCommand::SetMetronomeEnabled {
                    enabled: self.metronome_enabled,
                });
            }
            Message::CycleTimeSignature => {
                // Cycle through common time signatures
                let (num, den) = match (self.time_sig_num, self.time_sig_den) {
                    (4, 4) => (3, 4),
                    (3, 4) => (6, 8),
                    (6, 8) => (5, 4),
                    (5, 4) => (7, 8),
                    (7, 8) => (2, 4),
                    _ => (4, 4),
                };
                self.time_sig_num = num;
                self.time_sig_den = den;
                self.engine.send(AudioCommand::SetTimeSignature {
                    numerator: num,
                    denominator: den,
                });
            }
            Message::Tick => {
                while let Some(event) = self.engine.try_recv() {
                    self.handle_engine_event(event);
                }
            }
        }
        iced::Task::none()
    }

    fn handle_engine_event(&mut self, event: AudioEvent) {
        match event {
            AudioEvent::PlayheadMoved(pos) => {
                self.playhead = pos;
            }
            AudioEvent::SampleRateDetected { sample_rate } => {
                self.sample_rate = sample_rate;
            }
            AudioEvent::ClipImported {
                clip_id,
                track_id,
                start_sample,
                duration_samples,
                name,
            } => {
                self.clips.push(ClipState {
                    id: clip_id,
                    track_id,
                    start_sample,
                    duration_samples,
                    name,
                });
            }
            AudioEvent::TrackAdded { track_id } => {
                let order = self.next_track_order;
                self.next_track_order += 1;
                self.tracks.push(TrackState {
                    id: track_id,
                    name: format!("Track {}", track_id),
                    volume: 1.0,
                    muted: false,
                    order,
                    record_armed: false,
                    input_device_name: None,
                });
            }
            AudioEvent::TrackRemoved { track_id } => {
                self.tracks.retain(|t| t.id != track_id);
                self.clips.retain(|c| c.track_id != track_id);
            }
            AudioEvent::ClipDeleted { clip_id } => {
                self.clips.retain(|c| c.id != clip_id);
            }
            AudioEvent::ClipMoved {
                clip_id,
                new_start_sample,
                new_track_id,
            } => {
                if let Some(clip) = self.clips.iter_mut().find(|c| c.id == clip_id) {
                    clip.start_sample = new_start_sample;
                    clip.track_id = new_track_id;
                }
            }
            AudioEvent::Stopped => {
                self.playing = false;
                self.recording = false;
                self.playhead = 0;
            }
            AudioEvent::Error(e) => {
                eprintln!("Audio engine error: {}", e);
            }
            AudioEvent::InputDevicesListed {
                devices,
                default_name,
            } => {
                self.input_devices = devices;
                self.default_input_device_name = default_name;
            }
            AudioEvent::RecordingStarted { start_sample } => {
                self.recording = true;
                self.recording_start_sample = start_sample;
            }
            AudioEvent::RecordingFinished {
                clip_id,
                track_id,
                start_sample,
                duration_samples,
                name,
            } => {
                self.clips.push(ClipState {
                    id: clip_id,
                    track_id,
                    start_sample,
                    duration_samples,
                    name,
                });
                self.recording = false;
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::Tick)
    }

    fn view(&self) -> Element<'_, Message> {
        let transport = self.view_transport();
        let main_area = self.view_main_area();

        let content = column![transport, main_area].spacing(0);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::BG)),
                ..Default::default()
            })
            .into()
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
            button(text("⏸").size(18).color(theme::TEXT))
                .on_press(Message::Pause)
                .style(|_theme, status| theme::transport_button_style(status))
        } else {
            button(text("▶").size(18).color(theme::ACCENT))
                .on_press(Message::Play)
                .style(|_theme, status| theme::transport_button_style(status))
        };

        let stop_btn = button(text("⏹").size(18).color(theme::TEXT))
            .on_press(Message::Stop)
            .style(|_theme, status| theme::transport_button_style(status));

        let skip_back = button(text("⏪").size(16).color(theme::TEXT))
            .on_press(Message::SkipBack)
            .style(|_theme, status| theme::transport_button_style(status));

        let skip_fwd = button(text("⏩").size(16).color(theme::TEXT))
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
                if metronome_enabled {
                    let bg = match status {
                        iced::widget::button::Status::Hovered => iced::Color::from_rgb(0.15, 0.25, 0.15),
                        iced::widget::button::Status::Pressed => iced::Color::from_rgb(0.10, 0.20, 0.10),
                        _ => iced::Color::from_rgb(0.12, 0.20, 0.12),
                    };
                    iced::widget::button::Style {
                        background: Some(iced::Background::Color(bg)),
                        text_color: theme::METRONOME_ON,
                        border: iced::Border {
                            color: theme::METRONOME_ON,
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        ..Default::default()
                    }
                } else {
                    theme::transport_button_style(status)
                }
            });

        // Recording indicator
        let rec_indicator = if self.recording {
            text("● REC").size(14).color(theme::RECORD_RED)
        } else {
            text("").size(14)
        };

        let zoom_out = button(text("−").size(16).color(theme::TEXT))
            .on_press(Message::ZoomOut)
            .style(|_theme, status| theme::transport_button_style(status));

        let zoom_in = button(text("+").size(16).color(theme::TEXT))
            .on_press(Message::ZoomIn)
            .style(|_theme, status| theme::transport_button_style(status));

        let add_track = button(text("+ Track").size(14).color(theme::TEXT))
            .on_press(Message::AddTrack)
            .style(|_theme, status| theme::transport_button_style(status));

        let transport_row = row![
            Space::with_width(10),
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
            Space::with_width(Length::Fill),
            zoom_out,
            text("Zoom").size(12).color(theme::TEXT_DIM),
            zoom_in,
            Space::with_width(20),
            add_track,
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

    fn view_main_area(&self) -> Element<'_, Message> {
        let track_headers = self.view_track_headers();
        let timeline = self.view_timeline();

        let main = row![track_headers, timeline];

        container(main)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_track_headers(&self) -> Element<'_, Message> {
        let mut headers = column![].spacing(0);

        // Ruler header spacer
        headers = headers.push(
            container(Space::new(Length::Fill, 30)).style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::PANEL_DARK)),
                ..Default::default()
            }),
        );

        let mut sorted_tracks: Vec<&TrackState> = self.tracks.iter().collect();
        sorted_tracks.sort_by_key(|t| t.order);

        for track in sorted_tracks {
            let header = self.view_track_header(track.clone());
            headers = headers.push(header);
        }

        let content = scrollable(headers).height(Length::Fill);

        container(content)
            .width(180)
            .height(Length::Fill)
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

    fn view_track_header(&self, track: TrackState) -> Element<'_, Message> {
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

        let vol_slider = slider(0.0..=1.0, track.volume, {
            let id = track.id;
            move |v| Message::SetTrackVolume(id, v)
        })
        .width(80)
        .step(0.01);

        let vol_text = text(format!("{:.0}%", track.volume * 100.0))
            .size(11)
            .font(Font::MONOSPACE)
            .color(theme::TEXT_DIM);

        let import_btn = button(text("+").size(12).color(theme::TEXT))
            .on_press(Message::ImportFile(track.id))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2);

        let del_btn = button(text("×").size(12).color(theme::TEXT_DIM))
            .on_press(Message::RemoveTrack(track.id))
            .style(|_theme, status| theme::small_button_style(status))
            .padding(2);

        let top_row = row![
            name,
            Space::with_width(Length::Fill),
            rec_btn,
            mute_btn,
            import_btn,
            del_btn
        ]
        .spacing(4)
        .align_y(alignment::Vertical::Center);

        let bottom_row = row![vol_slider, vol_text]
            .spacing(4)
            .align_y(alignment::Vertical::Center);

        let mut content = column![top_row, bottom_row].spacing(4).padding(6);

        // Input device picker (shown when track is record-armed)
        if track.record_armed && !self.input_devices.is_empty() {
            let selected = track
                .input_device_name
                .as_ref()
                .and_then(|name| self.input_devices.iter().find(|d| &d.name == name))
                .cloned();

            let track_id = track.id;
            let device_picker = pick_list(
                self.input_devices.clone(),
                selected,
                move |device: InputDeviceInfo| {
                    Message::SetTrackInputDevice(track_id, Some(device.name))
                },
            )
            .placeholder("Select input...")
            .text_size(10)
            .width(Length::Fill);

            content = content.push(device_picker);
        }

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
            tracks: self.tracks.clone(),
            clips: self.clips.clone(),
            playhead: self.playhead,
            sample_rate: self.sample_rate,
            zoom: self.zoom,
            scroll_offset: self.scroll_offset,
            recording_tracks,
            recording_start_sample: self.recording_start_sample,
            bpm: self.bpm,
            time_sig_num: self.time_sig_num,
        };

        canvas(timeline_data)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

}
