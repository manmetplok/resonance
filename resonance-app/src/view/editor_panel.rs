//! Bottom MIDI editor panel shared by the Arrange and Compose views.
//! Hosts the piano roll (instrument tracks) or vocal roll (vocal tracks);
//! the variant is chosen once per paint by
//! [`Resonance::classify_editor_variant`] so a future third editor body
//! (e.g. a dedicated drum-cell editor) can be added by extending the
//! variant enum and one match arm.

use crate::message::*;
use crate::view::midi_editor::PianoRollCanvas;
use crate::theme;
use crate::view::compose::vocal_roll;
use iced::widget::{button, canvas, column, container, row, text, Space};
use iced::{alignment, Element, Length};
use resonance_audio::types::*;

/// Which editor body to render in the bottom MIDI editor panel. Picked
/// once per paint by [`Resonance::classify_editor_variant`].
pub(crate) enum EditorVariant {
    /// Standard piano roll — lavender accent, full 0..127 keyboard,
    /// shared with every non-vocal instrument track.
    Piano,
    /// Vocal roll — warm accent, voice-range-bounded keyboard,
    /// chord-context + phoneme strips, lyrics on note bodies, slur
    /// arcs, pitch-curve overlay.
    Vocal,
}

impl crate::Resonance {
    /// Pick the editor body to render for `track_id`. Centralised so
    /// the editor-panel dispatch can grow a third variant (e.g. a
    /// dedicated drum-cell editor) by adding a single arm here.
    pub(crate) fn classify_editor_variant(&self, track_id: TrackId) -> EditorVariant {
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
                let content_w = crate::view::midi_editor::KEYBOARD_WIDTH
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
                Space::new().width(Length::Fixed(12.0)),
                note_count,
                Space::new().width(Length::Fill),
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
}
