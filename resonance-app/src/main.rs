use iced::widget::{button, canvas, column, container, row, scrollable, slider, text, Space};
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
    sample_rate: u32,
    zoom: f32,          // pixels per second
    scroll_offset: f32, // horizontal scroll in pixels
    next_track_order: usize,
}

/// GUI-side track state.
#[derive(Debug, Clone)]
pub struct TrackState {
    pub id: TrackId,
    pub name: String,
    pub volume: f32,
    pub muted: bool,
    pub order: usize,
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

        let app = Self {
            engine,
            tracks: Vec::new(),
            clips: Vec::new(),
            playhead: 0,
            playing: false,
            sample_rate: 44100,
            zoom: 100.0,
            scroll_offset: 0.0,
            next_track_order: 0,
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
            AudioEvent::ClipImported {
                clip_id,
                track_id,
                duration_samples,
                name,
            } => {
                self.clips.push(ClipState {
                    id: clip_id,
                    track_id,
                    start_sample: self.playhead,
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
                self.playhead = 0;
            }
            AudioEvent::Error(e) => {
                eprintln!("Audio engine error: {}", e);
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
        let time_str = self.format_time(self.playhead);

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

        let time_display = text(time_str)
            .size(20)
            .font(Font::MONOSPACE)
            .color(theme::ACCENT);

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
            Space::with_width(20),
            time_display,
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

        let top_row = row![name, Space::with_width(Length::Fill), mute_btn, import_btn, del_btn]
            .spacing(4)
            .align_y(alignment::Vertical::Center);

        let bottom_row = row![vol_slider, vol_text]
            .spacing(4)
            .align_y(alignment::Vertical::Center);

        let content = column![top_row, bottom_row].spacing(4).padding(6);

        container(content)
            .width(Length::Fill)
            .height(theme::TRACK_HEIGHT)
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

    fn view_timeline(&self) -> Element<'_, Message> {
        let timeline_data = TimelineCanvas {
            tracks: self.tracks.clone(),
            clips: self.clips.clone(),
            playhead: self.playhead,
            sample_rate: self.sample_rate,
            zoom: self.zoom,
            scroll_offset: self.scroll_offset,
        };

        canvas(timeline_data)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn format_time(&self, samples: u64) -> String {
        let total_seconds = samples as f64 / self.sample_rate as f64;
        let minutes = (total_seconds / 60.0) as u64;
        let seconds = total_seconds % 60.0;
        format!("{:02}:{:05.2}", minutes, seconds)
    }
}
