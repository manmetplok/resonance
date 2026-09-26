//! Arrange-view geometry helpers that read across the track registry
//! and the arrange viewport.
//!
//! `track_id_at_arrange_y` maps a y-coordinate inside the arrange
//! canvas back to the track lane under the cursor. It lives here (not
//! on `TrackRegistry` directly) because it has to factor in the
//! viewport scroll offset and the arrange header height — knowledge that
//! belongs to the arrange view, not to the registry itself.
//!
//! Row resolution goes through the shared [`ArrangeRowLayout`](crate::view::arrange_layout)
//! (epic #36, doc #203) so it honours the heterogeneous 60/96 px row
//! pitch: a clip dragged over a group-header lane (or a collapsed group's
//! hidden member) resolves to *no* track, leaving the drag on its original
//! lane rather than snapping to a phantom row from the old uniform-pitch
//! division.

use resonance_audio::types::TrackId;

use crate::state::TrackState;
use crate::view::arrange_layout::{
    ArrangeAutomationRows, ArrangeRowKind, ArrangeRowLayout, ArrangeTakeRows,
};
use crate::{theme, Resonance};

impl Resonance {
    /// Tracks visible in the arrange view, sorted by `order`, excluding
    /// sub-tracks (rendered only in the mixer). Shared by the layout build
    /// and any caller that needs the arrange order.
    fn arrange_sorted_tracks(&self) -> Vec<&TrackState> {
        let mut sorted: Vec<&TrackState> = self
            .registry
            .tracks
            .iter()
            .filter(|t| t.sub_track.is_none())
            .collect();
        sorted.sort_by_key(|t| t.order);
        sorted
    }

    /// The fixed arrange-header height (ruler + section band + global-tracks
    /// shelf) above the first track lane, in canvas coordinates. Mirrors
    /// `TimelineCanvas::fixed_header_height` and the track-header column's
    /// chrome height so the three surfaces agree on where lane Y == 0 sits.
    pub(crate) fn arrange_header_offset(&self) -> f32 {
        let has_section_band = !self.compose.placements.is_empty();
        let section_band_h = if has_section_band {
            theme::SECTION_BAND_HEIGHT
        } else {
            0.0
        };
        let lane_labels_h = if self.viewport.global_tracks_expanded {
            theme::GLOBAL_TRACK_CHORD_HEIGHT
                + theme::GLOBAL_TRACK_TEMPO_HEIGHT
                + theme::GLOBAL_TRACK_SIG_HEIGHT
        } else {
            0.0
        };
        theme::RULER_HEIGHT
            + section_band_h
            + theme::GLOBAL_SHELF_HEADER_HEIGHT
            + lane_labels_h
    }

    /// The automation inputs to the shared arrange-row layout (doc #256):
    /// each arrange track's sorted lane ids from the live automation
    /// mirror, plus the transient expanded-track set. Shared by every
    /// layout build site so the canvas, header column and hit-testing
    /// always agree on the automation sub-rows.
    pub(crate) fn arrange_automation_rows(&self) -> ArrangeAutomationRows {
        ArrangeAutomationRows::collect(
            &self.automation,
            &self.arrange_sorted_tracks(),
            &self.ui.interaction.automation_expanded_tracks,
        )
    }

    /// The take-lane inputs to the shared arrange-row layout (epic #15,
    /// doc #165): each arrange track's `(group, take)` rows from the live
    /// take-group mirror, plus the transient expanded-track set. Shared by
    /// every layout build site so the canvas and header column always agree
    /// on the take sub-rows.
    pub(crate) fn arrange_take_rows(&self) -> ArrangeTakeRows {
        ArrangeTakeRows::collect(
            &self.take_groups,
            &self.arrange_sorted_tracks(),
            &self.ui.interaction.take_lane_expanded_tracks,
        )
    }

    /// Build the shared arrange-row layout — group-header rows, track rows,
    /// expanded automation sub-rows and expanded take sub-rows,
    /// collapse-aware — from the live registry, group, automation and take
    /// state.
    pub(crate) fn arrange_row_layout(&self) -> ArrangeRowLayout {
        ArrangeRowLayout::build_with_takes(
            &self.arrange_sorted_tracks(),
            &self.track_groups,
            &self.arrange_automation_rows(),
            &self.arrange_take_rows(),
        )
    }

    /// Find the visible track lane at the given y coordinate in the arrange
    /// view. Used by clip drag handlers to pick the target lane under the
    /// cursor. Sub-tracks are excluded (the arrange view hides them).
    ///
    /// Resolution goes through the shared [`ArrangeRowLayout`] so the
    /// variable 60/96 px row pitch and collapsed-member hiding are honoured.
    /// Returns `None` when the cursor is above the first row, below the
    /// last, or over a **group-header** lane (a group row is not a drop
    /// target — the caller keeps the clip on its original track), so a clip
    /// can never be dropped onto a phantom lane.
    pub(crate) fn track_id_at_arrange_y(&self, y: f32) -> Option<TrackId> {
        let header_offset = self.arrange_header_offset();
        let lane_y = y - header_offset + self.viewport.scroll_offset_y;
        if lane_y < 0.0 {
            return None;
        }
        match self.arrange_row_layout().row_at_y(lane_y)?.kind {
            ArrangeRowKind::Track(track_id) => Some(track_id),
            ArrangeRowKind::GroupHeader(_) => None,
            // An automation sub-row is not a clip drop target — like a
            // group header, dropping over it keeps the clip on its
            // original lane (confirmed with the dedicated lane-row editing
            // surface, todo #1097: lane rows carry envelopes, not clips).
            ArrangeRowKind::AutomationLane { .. } => None,
            // Nor is a take sub-row: a take is an alternate recording of
            // the slot, not a lane a clip can be dropped onto. Comping
            // gestures on these rows are todo #414's and go through the
            // canvas's own hit-testing, not this clip-drop path.
            ArrangeRowKind::TakeRow { .. } => None,
        }
    }
}
