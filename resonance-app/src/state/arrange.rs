//! Arrange-view geometry helpers that read across the track registry
//! and the arrange viewport.
//!
//! `track_id_at_arrange_y` maps a y-coordinate inside the arrange
//! canvas back to the track lane under the cursor. It lives here (not
//! on `TrackRegistry` directly) because it has to factor in the
//! viewport scroll offset and the global ruler height — knowledge that
//! belongs to the arrange view, not to the registry itself.

use resonance_audio::types::TrackId;

use crate::state::TrackState;
use crate::{theme, Resonance};

impl Resonance {
    /// Find the index in `self.registry.tracks` of the visible track at the
    /// given y coordinate in the arrange view. Used by clip drag handlers
    /// to pick the target lane under the cursor. Sub-tracks are excluded
    /// (the arrange view hides them).
    pub(crate) fn track_id_at_arrange_y(&self, y: f32) -> Option<TrackId> {
        let ruler_height = theme::RULER_HEIGHT;
        let track_idx = ((y - ruler_height + self.viewport.scroll_offset_y) / theme::TRACK_HEIGHT)
            .floor()
            .max(0.0) as usize;
        let mut sorted: Vec<&TrackState> = self
            .registry
            .tracks
            .iter()
            .filter(|t| t.sub_track.is_none())
            .collect();
        sorted.sort_by_key(|t| t.order);
        sorted.get(track_idx).map(|t| t.id)
    }
}
