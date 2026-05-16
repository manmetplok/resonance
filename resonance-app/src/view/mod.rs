/// View rendering for the Resonance application. The top-level dispatch
/// lives here; concrete surfaces are in sibling modules (transport,
/// mixer, compose, track_header, menus, settings).
pub(crate) mod bounce_dialog;
pub(crate) mod bounce_progress;
pub(crate) mod compose;
pub(crate) mod confirm_delete_track;
pub(crate) mod confirm_quit;
pub(crate) mod controls;
pub(crate) mod knob;
pub(crate) mod menus;
pub(crate) mod mixer;
pub(crate) mod settings;
pub(crate) mod startup;
pub(crate) mod track_header;
pub(crate) mod transport;
pub(crate) mod ui_caches;

use crate::message::*;
use crate::midi_editor::PianoRollCanvas;
use crate::state::*;
use crate::theme;
use crate::timeline::TimelineCanvas;
use crate::view::compose::vocal_roll;
use iced::widget::{button, canvas, column, container, row, stack, text, Space};
use iced::{alignment, Element, Length};
use resonance_audio::types::*;

/// Which editor body to render in the bottom MIDI editor panel. Picked
/// once per paint by [`Resonance::classify_editor_variant`].
enum EditorVariant {
    /// Standard piano roll — lavender accent, full 0..127 keyboard,
    /// shared with every non-vocal instrument track.
    Piano,
    /// Vocal roll — warm accent, voice-range-bounded keyboard,
    /// chord-context + phoneme strips, lyrics on note bodies, slur
    /// arcs, pitch-curve overlay.
    Vocal,
}

impl crate::Resonance {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let transport = transport::view_transport(self);
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
                        .on_press(Message::Ui(UiMessage::DismissError))
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
            .style(theme::base_bg)
            .into();

        if !self.io.has_active_project {
            stack![base, startup::view_startup_overlay(self)].into()
        } else if self.bounce_in_progress.is_some() {
            // The bounce progress modal sits above any other overlay
            // because it gates user input until the engine finishes the
            // current bounce — letting the quit-confirm or delete-track
            // dialog appear over it would invite the user into a state
            // change the engine isn't ready for.
            stack![
                base,
                bounce_progress::view_bounce_progress_overlay(self)
            ]
            .into()
        } else if self.confirm_quit.is_some() {
            stack![base, confirm_quit::view_confirm_quit_overlay(self)].into()
        } else if let Some(track_id) = self.confirm_delete_track {
            stack![
                base,
                confirm_delete_track::view_confirm_delete_track_overlay(self, track_id)
            ]
            .into()
        } else if self.bounce_dialog.is_some() {
            stack![base, bounce_dialog::view_bounce_dialog_overlay(self)].into()
        } else if self.mixer.settings_open {
            stack![base, settings::view_settings_overlay(self)].into()
        } else if self.mixer.add_track_menu_open {
            stack![base, menus::view_add_track_menu(self)].into()
        } else if self.compose.drumroll.manager_open
            && matches!(self.view_mode, ViewMode::Compose)
        {
            stack![base, compose::drum_groups_manager::view(self)].into()
        } else {
            base
        }
    }

    fn view_main_area(&self) -> Element<'_, Message> {
        let track_headers = track_header::view_track_headers(self);
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

    /// Pick the editor body to render for `track_id`. Centralised so
    /// the editor-panel dispatch can grow a third variant (e.g. a
    /// dedicated drum-cell editor) by adding a single arm here.
    fn classify_editor_variant(&self, track_id: TrackId) -> EditorVariant {
        let is_vocal = self
            .registry
            .tracks
            .iter()
            .find(|t| t.id == track_id)
            .map(|t| t.track_type == TrackType::Vocal)
            .unwrap_or(false);
        if is_vocal {
            EditorVariant::Vocal
        } else {
            EditorVariant::Piano
        }
    }

    /// Bottom MIDI editor panel shown whenever a clip is open in the piano
    /// roll. Used by both the Arrange and Compose tabs so inline editing
    /// works identically from either view. Classifies the clip into an
    /// [`EditorVariant`] once, then dispatches to the variant's body
    /// builder. The container chrome (close button row, fixed-width
    /// horizontal scroll, outer container border) is shared so a future
    /// third editor type only has to supply its own canvas + toolbar
    /// text.
    pub(crate) fn view_midi_editor_panel(&self) -> Option<Element<'_, Message>> {
        let editor_state = self.interaction.editing_midi_clip.as_ref()?;
        let clip = self
            .midi_clips
            .iter()
            .find(|c| c.id == editor_state.clip_id)?;

        let variant = self.classify_editor_variant(editor_state.track_id);
        let (body, toolbar_label, toolbar_accent, panel_height) = match variant {
            EditorVariant::Vocal => {
                let vocal_canvas = vocal_roll::build_canvas(self, clip)?;
                let label = format!("Vocal: {}  ·  {}", clip.name, vocal_canvas.voice_label);
                let extra_ticks: u64 =
                    4 * (self.transport.time_sig_num as u64) * TICKS_PER_QUARTER_NOTE;
                let content_ticks: u64 = clip.duration_ticks.saturating_add(extra_ticks);
                let content_w =
                    vocal_roll::VR_KEYBOARD_WIDTH + content_ticks as f32 * editor_state.zoom_x;
                let body = canvas(vocal_canvas)
                    .width(Length::Fixed(content_w))
                    .height(Length::Fill);
                let scrolled = iced::widget::Scrollable::with_direction(
                    body,
                    iced::widget::scrollable::Direction::Horizontal(
                        iced::widget::scrollable::Scrollbar::default(),
                    ),
                )
                .width(Length::Fill)
                .height(Length::Fill);
                (scrolled.into(), label, theme::WARM, 540)
            }
            EditorVariant::Piano => {
                let label = format!("MIDI: {}", clip.name);
                let extra_ticks: u64 =
                    4 * (self.transport.time_sig_num as u64) * TICKS_PER_QUARTER_NOTE;
                let content_ticks: u64 = clip.duration_ticks.saturating_add(extra_ticks);
                let content_w = crate::midi_editor::KEYBOARD_WIDTH
                    + content_ticks as f32 * editor_state.zoom_x;
                let piano_roll = canvas(PianoRollCanvas {
                    clip,
                    track_id: editor_state.track_id,
                    scroll_x: 0.0,
                    scroll_y: editor_state.scroll_y,
                    zoom_x: editor_state.zoom_x,
                    zoom_y: editor_state.zoom_y,
                    snap_ticks: editor_state.snap_ticks,
                    selected_note: editor_state.selected_note,
                    time_sig_num: self.transport.time_sig_num,
                })
                .width(Length::Fixed(content_w))
                .height(Length::Fill);
                let scrolled = iced::widget::Scrollable::with_direction(
                    piano_roll,
                    iced::widget::scrollable::Direction::Horizontal(
                        iced::widget::scrollable::Scrollbar::default(),
                    ),
                )
                .width(Length::Fill)
                .height(Length::Fill);
                let element: Element<'_, Message> = scrolled.into();
                (element, label, theme::ACCENT, 250)
            }
        };

        // Shared chrome: toolbar (label + optional note count + close)
        // and the outer container border. The toolbar's accent colour
        // is the variant's own — lavender for piano, warm for vocal.
        let close_btn = button(text("Close clip").size(12).color(theme::TEXT))
            .on_press(Message::MidiEditor(MidiEditorMessage::CloseMidiEditor))
            .style(|_theme, status| theme::transport_button_style(status))
            .padding([4, 8]);
        let editor_label = text(toolbar_label).size(12).color(toolbar_accent);
        let note_count = text(format!("{} notes", clip.notes.len()))
            .size(11)
            .color(theme::TEXT_3)
            .font(theme::MONO_FONT);
        let editor_toolbar = container(
            row![
                editor_label,
                Space::with_width(Length::Fixed(12.0)),
                note_count,
                Space::with_width(Length::Fill),
                close_btn,
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center)
            .padding([4, 8]),
        )
        .width(Length::Fill)
        .style(theme::panel_outlined);

        let editor_panel = column![editor_toolbar, body].spacing(0);
        Some(
            container(editor_panel)
                .width(Length::Fill)
                .height(panel_height)
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

    fn view_timeline(&self) -> Element<'_, Message> {
        let recording_tracks: Vec<TrackId> = if self.transport.recording {
            self.registry
                .tracks
                .iter()
                .filter(|t| t.record_armed)
                .map(|t| t.id)
                .collect()
        } else {
            Vec::new()
        };

        let timeline_data = TimelineCanvas {
            tracks: &self.registry.tracks,
            clips: &self.clips,
            playhead: self.transport.playhead,
            sample_rate: self.sample_rate,
            zoom: self.viewport.zoom,
            // Horizontal scrolling is now driven by the outer
            // `Scrollable` — the canvas always renders content from
            // sample-zero. The internal scroll_offset field is kept
            // (so `sample_to_x` and friends keep their signature) but
            // pinned to 0 from the view side.
            scroll_offset: 0.0,
            recording_tracks,
            recording_start_sample: self.transport.recording_start_sample,
            bpm: self.transport.bpm,
            time_sig_num: self.transport.time_sig_num,
            scroll_offset_y: self.viewport.scroll_offset_y,
            loop_enabled: self.transport.loop_enabled,
            loop_in: self.transport.loop_in,
            loop_out: self.transport.loop_out,
            selected_clip: self.interaction.selected_clip,
            midi_clips: &self.midi_clips,
            selected_midi_clip: self.interaction.selected_midi_clip,
            selected_track: self.interaction.selected_track,
            global_tracks_expanded: self.viewport.global_tracks_expanded,
            tempo_map: &self.tempo_map,
            selected_global_event: self.interaction.selected_global_event,
            section_placements: &self.compose.placements,
            section_definitions: &self.compose.definitions,
            selected_placement_id: self.compose.selected_placement_id,
        };

        // Fixed canvas width = full content width. With the canvas no
        // longer set to `Length::Fill`, its `bounds.size()` stays
        // stable across window resizes and `canvas::Cache` keeps
        // hitting instead of re-rasterizing every paint.
        let content_w = timeline_data.content_width_natural();
        let canvas_inner = canvas(timeline_data)
            .width(Length::Fixed(content_w))
            .height(Length::Fill);
        let canvas_el = iced::widget::Scrollable::with_direction(
            canvas_inner,
            iced::widget::scrollable::Direction::Horizontal(
                iced::widget::scrollable::Scrollbar::default(),
            ),
        )
        .width(Length::Fill)
        .height(Length::Fill);

        // Floating zoom buttons, anchored to the bottom-right corner of the
        // timeline. Using Length::Shrink so the overlay only hit-tests the
        // buttons themselves — clicks elsewhere pass through to the canvas.
        let zoom_out = button(
            theme::icon(theme::fa::MAGNIFYING_GLASS_MINUS)
                .size(12)
                .color(theme::TEXT),
        )
        .on_press(Message::Viewport(ViewportMessage::ZoomOut))
        .padding([6, 8])
        .style(|_theme, status| theme::floating_button_style(status));

        let zoom_in = button(
            theme::icon(theme::fa::MAGNIFYING_GLASS_PLUS)
                .size(12)
                .color(theme::TEXT),
        )
        .on_press(Message::Viewport(ViewportMessage::ZoomIn))
        .padding([6, 8])
        .style(|_theme, status| theme::floating_button_style(status));

        let zoom_group = row![zoom_out, zoom_in].spacing(4);

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
