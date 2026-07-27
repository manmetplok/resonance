//! Consolidated-overview strip for a **collapsed** group's lane (epic #36,
//! doc #203, todo #733).
//!
//! When a group is collapsed its member lanes drop out of the arrange
//! layout, so their clips are no longer drawn. To keep the content visible
//! at a glance, the group's 60 px lane is repainted as a consolidated
//! overview: every member clip — recursively, including nested sub-group
//! members — is flattened onto the single group lane and drawn as a short
//! tinted block at its bar span in the group's identity colour, with no
//! labels. Expanded groups instead keep the faint "spans all members"
//! identity wash from #731; that path never reaches this module.

use iced::widget::canvas;
use iced::{Color, Point, Size};

use crate::theme;
use resonance_common::track_group::TrackGroup;
use super::TimelineCanvas;

impl TimelineCanvas<'_> {
    /// Paint the consolidated overview onto a collapsed group's lane band.
    ///
    /// `lane_y` / `lane_height` are the group-header row's canvas rect (the
    /// caller has already filled the plain band and will draw the bottom
    /// rule on top). Every member clip — audio and MIDI, flattened
    /// recursively through nested sub-groups via
    /// [`get_all_member_ids`](crate::state::TrackGroupRegistry::get_all_member_ids)
    /// — becomes one tinted block at its timeline span, using the same
    /// `sample_to_x` mapping as the full clip bodies so the blocks line up
    /// bar-for-bar with the clips they stand in for.
    pub(in crate::view::timeline) fn draw_collapsed_group_overview(
        &self,
        frame: &mut canvas::Frame,
        group: &TrackGroup,
        lane_y: f32,
        lane_height: f32,
    ) {
        let (base, _wash, _line) = theme::group_identity_colors(group.identity_color);
        let members = self.track_groups.get_all_member_ids(group.id);
        if members.is_empty() {
            return;
        }

        let block_y = lane_y + theme::GROUP_OVERVIEW_INSET;
        let block_h = (lane_height - 2.0 * theme::GROUP_OVERVIEW_INSET).max(1.0);
        // Tinted, not solid: the strip says "there is content here", it is
        // not an editable clip body. Overlapping member clips compound the
        // alpha, which usefully reads as denser content.
        let tint = Color { a: 0.55, ..base };

        for clip in self.clips {
            if !members.contains(&clip.track_id) {
                continue;
            }
            self.overview_block(
                frame,
                clip.start_sample,
                clip.duration_samples,
                block_y,
                block_h,
                tint,
            );
        }
        for clip in self.midi_clips {
            if !members.contains(&clip.track_id) {
                continue;
            }
            // Tick length → absolute sample span, the same conversion the
            // full-lane MIDI clip body uses.
            let end = self.tempo_map.tick_to_abs_sample(
                clip.start_sample,
                clip.duration_ticks,
                self.sample_rate,
            );
            let duration_samples = end.saturating_sub(clip.start_sample);
            self.overview_block(
                frame,
                clip.start_sample,
                duration_samples,
                block_y,
                block_h,
                tint,
            );
        }
    }

    /// One overview block at the clip's sample span. Zero-width spans are
    /// skipped; horizontal culling is left to the caller's lane clip.
    fn overview_block(
        &self,
        frame: &mut canvas::Frame,
        start_sample: u64,
        duration_samples: u64,
        y: f32,
        h: f32,
        tint: Color,
    ) {
        let x0 = self.sample_to_x(start_sample);
        let x1 = self.sample_to_x(start_sample.saturating_add(duration_samples));
        let w = x1 - x0;
        if w <= 0.0 {
            return;
        }
        frame.fill_rectangle(Point::new(x0, y), Size::new(w, h), tint);
    }
}
