//! Click handling for the global-tracks shelf (chord, tempo, and signature
//! lanes). Isolated here because the logic is dense (~120 lines) and
//! unrelated to per-clip or ruler gestures.

use resonance_audio::types::bpm_at_bar;

use crate::message::*;
use crate::state;
use crate::theme;
use super::super::{TimelineCanvas, TimelineState};
use super::{captured, UpdateResult, DOUBLE_CLICK_MS};

impl TimelineCanvas<'_> {
    /// Compute BPM range for the tempo row graph (matches draw code).
    fn tempo_bpm_range(&self) -> (f32, f32) {
        let mut min_bpm = f32::MAX;
        let mut max_bpm = f32::MIN;
        for e in &self.tempo_map.tempo_points {
            min_bpm = min_bpm.min(e.bpm);
            max_bpm = max_bpm.max(e.bpm);
        }
        let range = (max_bpm - min_bpm).max(10.0);
        let pad = range * 0.15;
        (min_bpm - pad, max_bpm + pad)
    }

    /// Handle a click in the global-tracks lane area (below the shelf
    /// header). Single click on a tempo point starts a drag; double-click
    /// on empty space in the tempo lane adds a new event. Clicks in the
    /// chord lane are passive for now (the chord lane is read-only,
    /// driven by the compose sections).
    pub(in crate::view::timeline::input) fn handle_global_track_click(
        &self,
        state: &mut TimelineState,
        pos: iced::Point,
        _bounds: iced::Rectangle,
    ) -> UpdateResult {
        let ruler_height = theme::RULER_HEIGHT;
        let band_h = self.section_band_height();
        let shelf_header_h = theme::GLOBAL_SHELF_HEADER_HEIGHT;
        let chord_h = theme::GLOBAL_TRACK_CHORD_HEIGHT;
        let tempo_h = theme::GLOBAL_TRACK_TEMPO_HEIGHT;
        let sig_h = theme::GLOBAL_TRACK_SIG_HEIGHT;

        let chord_top = ruler_height + band_h + shelf_header_h;
        let tempo_top = chord_top + chord_h;
        let sig_top = tempo_top + tempo_h;

        let row_h = tempo_h; // BPM graph math assumes tempo-lane height

        let in_tempo = pos.y >= tempo_top && pos.y < tempo_top + tempo_h;
        let in_sig = pos.y >= sig_top && pos.y < sig_top + sig_h;
        let _in_chord = pos.y >= chord_top && pos.y < chord_top + chord_h;

        let bar = self.x_to_bar(pos.x);

        // For step changes (two events at same bar) pick the closest point
        // by y-distance so both are individually draggable.
        if in_tempo {
            let (lo, hi) = self.tempo_bpm_range();
            let graph_top = tempo_top + 3.0;
            let graph_bot = tempo_top + row_h - 3.0;
            let graph_h = graph_bot - graph_top;

            let mut best: Option<(usize, f32)> = None; // (index, distance²)
            for (i, event) in self.tempo_map.tempo_points.iter().enumerate() {
                let sample = self.tempo_map.bar_to_sample(event.bar);
                let ex = self.sample_to_x(sample);
                let ey = graph_bot - ((event.bpm - lo) / (hi - lo)) * graph_h;
                let dx = pos.x - ex;
                let dy = pos.y - ey;
                let dist2 = dx * dx + dy * dy;
                if dx.abs() < 10.0
                    && dy.abs() < 12.0
                    && best.is_none_or(|(_, d)| dist2 < d)
                {
                    best = Some((i, dist2));
                }
            }
            if let Some((i, _)) = best {
                use super::TempoDrag;
                state.tempo_drag = Some(TempoDrag {
                    index: i,
                    original_bpm: self.tempo_map.tempo_points[i].bpm,
                    anchor_y: pos.y,
                });
                return captured(Message::GlobalTrack(GlobalTrackMessage::StartTempoDrag(i)));
            }
            // Double-click detection for adding new tempo events.
            let now = std::time::Instant::now();
            let is_double = state
                .last_global_click
                .map(|(t, k)| {
                    k == state::GlobalTrackKind::Tempo
                        && now.duration_since(t).as_millis() <= DOUBLE_CLICK_MS
                })
                .unwrap_or(false);
            state.last_global_click = Some((now, state::GlobalTrackKind::Tempo));
            if is_double {
                state.last_global_click = None;
                // Add at the interpolated BPM for this bar so the point
                // appears on the current line.
                let bpm = bpm_at_bar(bar as f64, &self.tempo_map.tempo_points) as f32;
                return captured(Message::GlobalTrack(GlobalTrackMessage::AddTempoEvent {
                    bar,
                    bpm,
                }));
            }
            // Single click on empty space → deselect.
            return captured(Message::GlobalTrack(GlobalTrackMessage::SelectEvent(None)));
        }

        if in_sig {
            for (i, event) in self.tempo_map.signature_points.iter().enumerate() {
                let sample = self.tempo_map.bar_to_sample(event.bar);
                let ex = self.sample_to_x(sample);
                if (pos.x - ex).abs() < 8.0 {
                    return captured(Message::GlobalTrack(GlobalTrackMessage::SelectEvent(Some(
                        state::SelectedGlobalEvent {
                            kind: state::GlobalTrackKind::Signature,
                            index: i,
                        },
                    ))));
                }
            }
            let now = std::time::Instant::now();
            let is_double = state
                .last_global_click
                .map(|(t, k)| {
                    k == state::GlobalTrackKind::Signature
                        && now.duration_since(t).as_millis() <= DOUBLE_CLICK_MS
                })
                .unwrap_or(false);
            state.last_global_click = Some((now, state::GlobalTrackKind::Signature));
            if is_double {
                state.last_global_click = None;
                return captured(Message::GlobalTrack(
                    GlobalTrackMessage::AddSignatureEvent {
                        bar,
                        numerator: self.time_sig_num,
                        denominator: 4,
                    },
                ));
            }
            return captured(Message::GlobalTrack(GlobalTrackMessage::SelectEvent(None)));
        }

        Some(iced::widget::canvas::Action::capture())
    }
}
