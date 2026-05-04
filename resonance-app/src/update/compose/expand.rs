//! Expanded-piano-roll viewport state for the Compose tab: which track
//! is open, plus its scroll offsets and vertical zoom.

use resonance_audio::types::TrackId;

use crate::compose::SelectedLane;

pub(super) fn handle_expand(r: &mut crate::Resonance, track_id: TrackId) {
    if r.compose.expanded_track_id == Some(track_id) {
        r.compose.expanded_track_id = None;
        return;
    }
    r.compose.expanded_track_id = Some(track_id);
    let is_drum = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .map(|t| t.instrument_type == crate::state::InstrumentType::Drum)
        .unwrap_or(false);
    r.compose.selected_lane = if is_drum {
        SelectedLane::Drums(track_id)
    } else {
        SelectedLane::Instrument(track_id)
    };
    r.compose.expanded_scroll_x = 0.0;
    r.compose.expanded_scroll_y = 40.0 * r.compose.expanded_zoom_y;
}

pub(super) fn handle_collapse(r: &mut crate::Resonance) {
    r.compose.expanded_track_id = None;
}

pub(super) fn handle_scroll_x(r: &mut crate::Resonance, delta: f32) {
    r.compose.expanded_scroll_x = (r.compose.expanded_scroll_x + delta).max(0.0);
}

pub(super) fn handle_scroll_y(r: &mut crate::Resonance, delta: f32) {
    r.compose.expanded_scroll_y = (r.compose.expanded_scroll_y + delta).max(0.0);
}

pub(super) fn handle_zoom_y(r: &mut crate::Resonance, delta: f32) {
    r.compose.expanded_zoom_y = (r.compose.expanded_zoom_y + delta).clamp(4.0, 40.0);
}
