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
    /// The fully-populated [`TimelineCanvas`] view-model for the current
    /// app state. Split from [`view_timeline`](Self::view_timeline) so the
    /// canvas the user sees and the test-support event / fingerprint hooks
    /// (`test_support.rs`) are built from the exact same data.
    pub(crate) fn timeline_canvas_data(&self) -> TimelineCanvas<'_> {
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
        let frozen_tracks: std::collections::HashSet<TrackId> = self
            .registry
            .tracks
            .iter()
            .filter(|t| self.freeze.status(t.id).is_frozen())
            .map(|t| t.id)
            .collect();

        TimelineCanvas {
            keys_blocked: self.canvas_keys_blocked(),
            keymap: &self.ui.keymap,
            key_grant: self.ui.interaction.timeline_key_grant,
            tracks: &self.registry.tracks,
            track_groups: &self.track_groups,
            clips: &self.clips,
            playhead: self.transport.playhead,
            sample_rate: self.sample_rate,
            zoom: self.viewport.zoom,
            recording_tracks,
            recording_start_sample: self.transport.recording_start_sample,
            bpm: self.transport.bpm,
            time_sig_num: self.transport.time_sig_num,
            scroll_offset_y: self.viewport.scroll_offset_y,
            loop_enabled: self.transport.loop_enabled,
            loop_in: self.transport.loop_in,
            loop_out: self.transport.loop_out,
            selected_clip: self.ui.interaction.selected_clip,
            midi_clips: &self.midi_clips,
            selected_midi_clip: self.ui.interaction.selected_midi_clip,
            selected_track: self.ui.interaction.selected_track,
            global_tracks_expanded: self.viewport.global_tracks_expanded,
            tempo_map: &self.tempo_map,
            selected_global_event: self.ui.interaction.selected_global_event,
            section_placements: &self.compose.placements,
            section_definitions: &self.compose.definitions,
            selected_placement_id: self.compose.selected_placement_id,
            automation: &self.automation,
            device_param_labels: crate::view::timeline::automation::device_param_labels(
                &self.automation,
                &self.devices.external_instruments,
                &self.devices.registry,
            ),
            markers: self.markers.as_slice(),
            selected_marker_id: self.ui.interaction.selected_marker_id,
            frozen_tracks,
            drag: self.media.drag_placement.as_ref(),
            automation_expanded_tracks: &self.ui.interaction.automation_expanded_tracks,
            take_groups: &self.take_groups,
            take_lane_expanded_tracks: &self.ui.interaction.take_lane_expanded_tracks,
            layout_memo: Default::default(),
            content_fingerprint_memo: Default::default(),
            visible_viewport: Default::default(),
        }
    }

    /// Test-only (view-performance batch): the memoized arrange layout —
    /// warmed through the same accessor every draw / hover call uses —
    /// next to an uncached rebuild, so the memo's null test can assert
    /// they never diverge. Lives here rather than in `test_support`
    /// because the memo is a `view::timeline` implementation detail.
    #[doc(hidden)]
    pub fn test_timeline_layout_memo_pair(
        &self,
    ) -> (
        crate::view::arrange_layout::ArrangeRowLayout,
        crate::view::arrange_layout::ArrangeRowLayout,
    ) {
        let canvas = self.timeline_canvas_data();
        // Warm the memo through a production consumer first, so the test
        // exercises the reuse path and not just a first fill.
        let _ = canvas.content_height_px();
        (canvas.arrange_layout().clone(), canvas.build_arrange_layout())
    }

    pub(crate) fn view_timeline(&self) -> Element<'_, Message> {
        let timeline_data = self.timeline_canvas_data();

        // Fixed canvas width = full content width. With the canvas no
        // longer set to `Length::Fill`, its `bounds.size()` stays
        // stable across window resizes and `canvas::Cache` keeps
        // hitting instead of re-rasterizing every paint.
        let content_w = timeline_data.content_width_natural();
        // The probe cell is shared between the canvas program (which
        // reads it to cull the cached pass to the visible viewport) and
        // the `ViewportProbe` wrapper below (which writes the viewport
        // into it right before every draw) — the `canvas::Program` API
        // itself never sees the outer `Scrollable`'s viewport.
        let visible_viewport = timeline_data.visible_viewport.clone();
        let canvas_inner = canvas(timeline_data)
            .width(Length::Fixed(content_w))
            .height(Length::Fill);
        let canvas_probe = crate::view::timeline::viewport_probe::ViewportProbe::new(
            canvas_inner,
            visible_viewport,
        );
        let canvas_el = iced::widget::Scrollable::with_direction(
            canvas_probe,
            iced::widget::scrollable::Direction::Horizontal(
                iced::widget::scrollable::Scrollbar::default(),
            ),
        )
        .id(crate::state::ARRANGE_SCROLL_ID)
        .on_scroll(|vp| {
            Message::Viewport(ViewportMessage::ArrangeScrolled {
                offset_x: vp.absolute_offset().x,
                visible_width: vp.bounds().width,
                content_width: vp.content_bounds().width,
            })
        })
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
