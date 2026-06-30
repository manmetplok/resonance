//! Arrange-view timeline panel: the main horizontally-scrolled canvas
//! that renders tracks, clips, the playhead, recording overlays, and
//! the loop markers, plus the floating zoom buttons anchored to the
//! bottom-right of the timeline.

use crate::message::*;
use crate::theme;
use crate::view::timeline::TimelineCanvas;
use iced::widget::{button, canvas, column, container, row, stack, Space};
use iced::{Element, Length};
use resonance_audio::types::*;

impl crate::Resonance {
    pub(crate) fn view_timeline(&self) -> Element<'_, Message> {
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

        // Tracks playing back from a freeze cache have no editable sample
        // source, so their audio clips render the "unsupported" surface
        // (hatch, no fade handles) while gain still applies — design #153.
        let frozen_tracks: Vec<TrackId> = self
            .registry
            .tracks
            .iter()
            .filter(|t| self.freeze.status(t.id).is_frozen())
            .map(|t| t.id)
            .collect();

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
            markers: self.markers.as_slice(),
            selected_marker_id: self.interaction.selected_marker_id,
            frozen_tracks,
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
                Space::new().height(Length::Fill),
                row![Space::new().width(Length::Fill), zoom_group],
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
