//! Timeline canvas: the arrangement view for tracks, audio clips, and
//! MIDI clips. The canvas's three concerns are split across files:
//!
//! - this file: [`TimelineCanvas`] struct, small geometry helpers, and
//!   the [`canvas::Program`] impl that orchestrates per-event dispatch
//!   and per-frame drawing.
//! - [`input`](crate::view::timeline::input): pointer / wheel /
//!   keyboard event handling and the [`TimelineState`] drag tracker.
//! - [`draw`](crate::view::timeline::draw): pure-draw routines for
//!   the ruler, grid, global tracks, and clips.
//! - [`snap`](crate::view::timeline::snap): the snap-to-grid helpers
//!   shared with the clip-drag and seek paths.
use std::time::Instant;

use iced::widget::canvas;
use iced::{keyboard, mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use crate::message::*;
use crate::state::{self, ClipState, MidiClipState, TrackState};
use crate::theme;
use self::input::{ClipInteraction, TempoDrag};

use resonance_audio::types::{ClipId, TempoMap, TrackId};

pub mod draw;
pub mod hit_test;
pub mod input;
pub mod scrollbar;
pub mod snap;

// Snap helpers are external public API for this canvas — re-export them
// from the snap submodule so existing call sites keep working.
pub use self::snap::{snap_sample_to_grid, snap_sample_to_grid_tempo};

/// Data passed to the timeline canvas for rendering.
#[derive(Debug)]
pub struct TimelineCanvas<'a> {
    pub tracks: &'a [TrackState],
    pub clips: &'a [ClipState],
    pub playhead: u64,
    pub sample_rate: u32,
    pub zoom: f32,
    pub scroll_offset: f32,
    pub recording_tracks: Vec<TrackId>,
    pub recording_start_sample: u64,
    pub bpm: f32,
    pub time_sig_num: u8,
    pub scroll_offset_y: f32,
    pub loop_enabled: bool,
    pub loop_in: u64,
    pub loop_out: u64,
    pub selected_clip: Option<ClipId>,
    pub midi_clips: &'a [MidiClipState],
    pub selected_midi_clip: Option<ClipId>,
    pub selected_track: Option<TrackId>,
    pub global_tracks_expanded: bool,
    pub tempo_map: &'a TempoMap,
    pub selected_global_event: Option<crate::state::SelectedGlobalEvent>,
    /// Compose section placements + definitions, threaded so the
    /// section-pill band can sit above the lanes. Empty slices => no band.
    pub section_placements: &'a [crate::compose::SectionPlacementState],
    pub section_definitions: &'a [crate::compose::SectionDefinitionState],
    pub selected_placement_id: Option<u64>,
}

impl TimelineCanvas<'_> {
    /// Height of the always-visible global-shelf header strip (the
    /// "GLOBAL · 6/8 · 90 BPM · …" summary bar). Present regardless of
    /// the expanded state — the chat brief made the shelf "collapsable
    /// above the regular tab", so the summary line is always there.
    pub(crate) fn global_shelf_header_height(&self) -> f32 {
        theme::GLOBAL_SHELF_HEADER_HEIGHT
    }

    /// Height of the *expanded* global-tracks lane area — three rows
    /// stacked (chords + tempo + signature). Returns 0.0 when the
    /// shelf is collapsed so the lane area drops to zero and only the
    /// header strip stays.
    pub(crate) fn global_tracks_lanes_height(&self) -> f32 {
        if self.global_tracks_expanded {
            theme::GLOBAL_TRACK_CHORD_HEIGHT
                + theme::GLOBAL_TRACK_TEMPO_HEIGHT
                + theme::GLOBAL_TRACK_SIG_HEIGHT
        } else {
            0.0
        }
    }

    /// Total height of the global-tracks region (header strip + lanes).
    /// Used by `fixed_header_height` and the track-header column to
    /// keep their Y offsets in sync.
    pub(crate) fn global_tracks_height(&self) -> f32 {
        self.global_shelf_header_height() + self.global_tracks_lanes_height()
    }

    /// Height of the section-pill band sitting under the ruler. Returns
    /// 0.0 when no sections are placed so empty projects don't take a
    /// vertical hit.
    pub(crate) fn section_band_height(&self) -> f32 {
        if self.section_placements.is_empty() {
            0.0
        } else {
            theme::SECTION_BAND_HEIGHT
        }
    }

    /// Total fixed header height: ruler + section band + global tracks area.
    /// This is the Y offset where regular track rows begin.
    pub(crate) fn fixed_header_height(&self) -> f32 {
        theme::RULER_HEIGHT + self.section_band_height() + self.global_tracks_height()
    }

    /// Convert a sample position to pixel x coordinate.
    pub(crate) fn sample_to_x(&self, sample: u64) -> f32 {
        (sample as f64 / self.sample_rate as f64) as f32 * self.zoom - self.scroll_offset
    }

    /// Rightmost pixel needed to show all content (clips + MIDI clips).
    /// Always returns at least `viewport_width * 1.5` so users can scroll a
    /// bit past the last clip, and never less than `viewport_width` itself.
    pub(crate) fn content_width_px(&self, viewport_width: f32) -> f32 {
        let mut max_sample: u64 = 0;
        for c in self.clips {
            let end = c.start_sample + c.duration_samples;
            if end > max_sample {
                max_sample = end;
            }
        }
        for c in self.midi_clips {
            let end = self.tempo_map.tick_to_abs_sample(
                c.start_sample,
                c.duration_ticks,
                self.sample_rate,
            );
            if end > max_sample {
                max_sample = end;
            }
        }
        let content = (max_sample as f64 / self.sample_rate as f64) as f32 * self.zoom;
        content.max(viewport_width * 1.5).max(viewport_width)
    }

    /// Same as `content_width_px` but adds a fixed trailing pad in bars
    /// instead of inflating to 1.5× a viewport that the canvas never
    /// directly knows. Used by `view_timeline` to size the canvas
    /// inside the horizontal `Scrollable` — bounding the canvas to its
    /// own natural size lets `canvas::Cache` hit across window resizes
    /// (the cache invalidates on `bounds.size()` changes).
    pub(crate) fn content_width_natural(&self) -> f32 {
        let mut max_sample: u64 = 0;
        for c in self.clips {
            let end = c.start_sample + c.duration_samples;
            if end > max_sample {
                max_sample = end;
            }
        }
        for c in self.midi_clips {
            let end = self.tempo_map.tick_to_abs_sample(
                c.start_sample,
                c.duration_ticks,
                self.sample_rate,
            );
            if end > max_sample {
                max_sample = end;
            }
        }
        let content = (max_sample as f64 / self.sample_rate as f64) as f32 * self.zoom;
        // 8 bars of trailing pad so the user can drop new clips just
        // past the last existing one without immediately scrolling out
        // of canvas. Floor of 800 px keeps empty projects usable.
        let seconds_per_bar =
            self.time_sig_num as f32 * 60.0 / self.bpm.max(1.0);
        let pad = 8.0 * seconds_per_bar * self.zoom;
        (content + pad).max(800.0)
    }

    /// Total vertical content height (tracks + ruler + global tracks).
    /// Excludes sub-tracks since the arrange view hides them entirely.
    pub(crate) fn content_height_px(&self) -> f32 {
        let visible = self.tracks.iter().filter(|t| t.sub_track.is_none()).count();
        self.fixed_header_height() + visible as f32 * theme::TRACK_HEIGHT
    }

    /// Tracks visible in the arrange view, sorted by `order`. Excludes
    /// sub-tracks (rendered only in the mixer view).
    pub(super) fn visible_tracks_sorted(&self) -> Vec<&TrackState> {
        hit_test::sorted_arrange_tracks(self.tracks)
    }
}

/// Local state for the timeline canvas, tracking active drag operations.
#[derive(Debug, Default)]
pub struct TimelineState {
    pub(super) dragging_loop: bool,
    pub(super) clip_interaction: Option<ClipInteraction>,
    pub(super) last_reported_width: f32,
    pub(super) last_reported_height: f32,
    pub(super) last_reported_content_width: f32,
    pub(super) last_reported_content_height: f32,
    /// Tracks the most recent click on a MIDI clip for double-click detection.
    pub(super) last_midi_click: Option<(Instant, ClipId)>,
    /// Horizontal scrollbar drag in progress. Stores the x-offset of the
    /// grab point relative to the left edge of the thumb (in track pixels).
    pub(super) h_scrollbar_grab: Option<f32>,
    /// Vertical scrollbar drag in progress (y-offset within the thumb).
    pub(super) v_scrollbar_grab: Option<f32>,
    /// Tracks the most recent click on a global track for double-click detection.
    pub(super) last_global_click: Option<(Instant, state::GlobalTrackKind)>,
    /// Active tempo-event drag.
    pub(super) tempo_drag: Option<TempoDrag>,
    /// Geometry cache — re-runs the draw closure only when the cached
    /// fingerprint mismatches. Skips a full redraw on every hover /
    /// sibling-update event, which is most of them.
    pub(super) cache: canvas::Cache,
    /// Snapshot of the input fields that affect the rendered geometry
    /// at the moment the cache was last filled. The draw routine
    /// compares the current frame's fingerprint to this and invalidates
    /// the cache when any field changes.
    pub(super) cache_fingerprint: std::cell::Cell<TimelineFingerprint>,
}

/// Compact summary of the data the timeline reads. When any of these
/// values changes between frames, the canvas geometry needs a redraw.
/// `Default` returns a sentinel value the first frame can never match,
/// so the very first draw fills the cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TimelineFingerprint {
    pub clips_len: usize,
    pub midi_clips_len: usize,
    pub tracks_len: usize,
    // NOTE: playhead and recording_start_sample are intentionally NOT
    // in the fingerprint. They change continuously during playback /
    // recording and would invalidate the cache every frame, defeating
    // the whole purpose. The playhead overlay (line + tab) and the
    // recording overlay are drawn as a separate uncached Geometry in
    // `Program::draw` below.
    pub zoom_bits: u32,
    pub scroll_x_bits: u32,
    pub scroll_y_bits: u32,
    pub recording_count: usize,
    pub loop_enabled: bool,
    pub loop_in: u64,
    pub loop_out: u64,
    pub selected_clip: Option<ClipId>,
    pub selected_midi_clip: Option<ClipId>,
    pub selected_track: Option<TrackId>,
    pub global_tracks_expanded: bool,
    pub tempo_points: usize,
    pub signature_points: usize,
    /// Hash of every tempo event's `(bar, bpm)` so the cache invalidates
    /// when an *existing* event's value changes (drag, transport-side
    /// BPM commit, pick_list edit). Just tracking `tempo_points.len()`
    /// would miss in-place edits and leave the canvas curve stale.
    pub tempo_events_hash: u64,
    /// Same idea for signature events: their `(bar, numerator,
    /// denominator)` so pill markers + label text redraw on edit.
    pub signature_events_hash: u64,
    /// Currently selected tempo/signature event. The draw routine
    /// recolors the selected dot / pill marker with the accent so this
    /// has to enter the fingerprint, otherwise a fresh click "lands"
    /// in state but doesn't repaint until something else does.
    pub selected_global_event: Option<state::SelectedGlobalEvent>,
    pub bpm_bits: u32,
    pub time_sig_num: u8,
    pub section_placements_len: usize,
    pub section_definitions_len: usize,
    pub selected_placement_id: Option<u64>,
    /// Sum of every section definition's chord count. Drives the
    /// chord-lane redraw inside the global shelf — without this the
    /// canvas cache would hold a stale chord layout after a chord is
    /// added / removed / re-rolled inside Compose.
    pub section_chord_total: usize,
}

impl<'a> TimelineCanvas<'a> {
    fn fingerprint(&self) -> TimelineFingerprint {
        // Hash the full tempo + signature event content so any
        // *in-place* edit (drag, pick_list change, transport-bar
        // commit) invalidates the cache and the curve / pill markers
        // redraw on the next frame.
        use std::hash::{Hash, Hasher};
        let mut th = std::collections::hash_map::DefaultHasher::new();
        for e in &self.tempo_map.tempo_points {
            e.bar.hash(&mut th);
            e.bpm.to_bits().hash(&mut th);
        }
        let tempo_events_hash = th.finish();
        let mut sh = std::collections::hash_map::DefaultHasher::new();
        for e in &self.tempo_map.signature_points {
            e.bar.hash(&mut sh);
            e.numerator.hash(&mut sh);
            e.denominator.hash(&mut sh);
        }
        let signature_events_hash = sh.finish();

        TimelineFingerprint {
            clips_len: self.clips.len(),
            midi_clips_len: self.midi_clips.len(),
            tracks_len: self.tracks.len(),
            zoom_bits: self.zoom.to_bits(),
            scroll_x_bits: self.scroll_offset.to_bits(),
            scroll_y_bits: self.scroll_offset_y.to_bits(),
            recording_count: self.recording_tracks.len(),
            loop_enabled: self.loop_enabled,
            loop_in: self.loop_in,
            loop_out: self.loop_out,
            selected_clip: self.selected_clip,
            selected_midi_clip: self.selected_midi_clip,
            selected_track: self.selected_track,
            global_tracks_expanded: self.global_tracks_expanded,
            tempo_points: self.tempo_map.tempo_points.len(),
            signature_points: self.tempo_map.signature_points.len(),
            tempo_events_hash,
            signature_events_hash,
            selected_global_event: self.selected_global_event,
            bpm_bits: self.bpm.to_bits(),
            time_sig_num: self.time_sig_num,
            section_placements_len: self.section_placements.len(),
            section_definitions_len: self.section_definitions.len(),
            selected_placement_id: self.selected_placement_id,
            section_chord_total: self
                .section_definitions
                .iter()
                .map(|d| d.chords.len())
                .sum(),
        }
    }
}

impl canvas::Program<Message> for TimelineCanvas<'_> {
    type State = TimelineState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let result = match event {
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                self.handle_wheel(*delta, bounds, cursor)
            }
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                self.handle_press(state, bounds, cursor)
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                self.handle_move(state, bounds, cursor)
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                self.handle_release(state)
            }
            iced::Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
                self.handle_key(key)
            }
            _ => None,
        };
        if result.is_some() {
            return result;
        }
        if let Some(msg) = self.report_viewport(state, bounds) {
            return Some(canvas::Action::publish(msg));
        }
        None
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        self.hover_interaction(state, bounds, cursor)
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        // Cache invalidation: re-runs the body only when our fingerprint
        // changes. Pure hover/sibling redraws hit the cached geometry.
        // `playhead` and `recording_start_sample` are intentionally
        // excluded from the fingerprint — the playhead line + tab and
        // the per-track recording overlay are drawn in a second
        // uncached pass below so they update every frame without
        // invalidating the rest of the timeline.
        let fp = self.fingerprint();
        if state.cache_fingerprint.get() != fp {
            state.cache.clear();
            state.cache_fingerprint.set(fp);
        }
        let cached = state.cache.draw(renderer, bounds.size(), |frame| {
            self.draw_into(frame, bounds);
        });
        let mut overlay = canvas::Frame::new(renderer, bounds.size());
        self.draw_overlay_into(&mut overlay, bounds);
        vec![cached, overlay.into_geometry()]
    }
}

impl<'a> TimelineCanvas<'a> {
    /// Run the full draw routine onto the given frame. Split out so the
    /// `Program::draw` impl can wrap it in `canvas::Cache`.
    fn draw_into(&self, frame: &mut canvas::Frame, bounds: Rectangle) {
        let ruler_height = theme::RULER_HEIGHT;
        let header_height = self.fixed_header_height();
        let y_off = self.scroll_offset_y;

        // Draw ruler background
        frame.fill_rectangle(
            Point::new(0.0, 0.0),
            Size::new(bounds.width, ruler_height),
            theme::BG_1,
        );

        // Section-pill band — sits under the ruler when at least one
        // compose section is placed. Render before global tracks so the
        // global tracks shift down accordingly.
        let band_top = ruler_height;
        let band_height = self.section_band_height();
        if band_height > 0.0 {
            self.draw_section_band(frame, bounds.width, band_top, band_height);
        }

        // Draw global tracks area (tempo + time signature) between the
        // section band and the regular tracks.
        self.draw_global_tracks(frame, bounds.width, band_top + band_height);

        // Draw track backgrounds. Only non-sub-tracks are rendered; the
        // mixer view is where sub-track lanes live.
        let sorted_tracks = self.visible_tracks_sorted();
        let track_area_height = sorted_tracks.len() as f32 * theme::TRACK_HEIGHT;

        // Everything inside the lane region — track rows, grid lines,
        // clips, and the loop in/out dim overlays — is clipped to the
        // area below the fixed header. Without this, a track straddling
        // the header_height boundary (sub-row vertical scroll, or a
        // partial top row when scrolled mid-row) paints its background
        // and clip body over the ruler / section-pill band / global
        // tracks above. Ruler labels and loop / playhead markers stay
        // outside the clip on purpose so their handles can sit on top
        // of the ruler.
        let lane_clip = Rectangle {
            x: 0.0,
            y: header_height,
            width: bounds.width,
            height: (bounds.height - header_height).max(0.0),
        };
        let loop_dim_color = Color::from_rgba(0.0, 0.0, 0.0, 0.15);
        let loop_total_height_below_header =
            (track_area_height - y_off).max(bounds.height - header_height);
        let loop_dim_height = loop_total_height_below_header.max(0.0);
        frame.with_clip(lane_clip, |frame| {
            for (i, track) in sorted_tracks.iter().enumerate() {
                let y = header_height + i as f32 * theme::TRACK_HEIGHT - y_off;

                // Skip tracks entirely above or below the visible area
                if y + theme::TRACK_HEIGHT < header_height || y > bounds.height {
                    continue;
                }

                let is_selected = self.selected_track == Some(track.id);
                let bg = if is_selected {
                    theme::BG_2
                } else if i % 2 == 0 {
                    theme::BG
                } else {
                    theme::PANEL_DARK
                };
                frame.fill_rectangle(
                    Point::new(0.0, y),
                    Size::new(bounds.width, theme::TRACK_HEIGHT),
                    bg,
                );

                // Recording overlay is drawn in the uncached overlay pass
                // — see `draw_overlay_into`. It grows with `playhead`,
                // which would otherwise invalidate the cache every frame.

                // Track separator line
                frame.fill_rectangle(
                    Point::new(0.0, y + theme::TRACK_HEIGHT - 1.0),
                    Size::new(bounds.width, 1.0),
                    theme::LINE_2,
                );
            }

            // Draw bar/beat grid lines through track area
            self.draw_grid_lines(
                frame,
                bounds.width,
                header_height,
                track_area_height,
                y_off,
            );

            // Draw audio clips
            for clip in self.clips {
                self.draw_clip(
                    frame,
                    clip,
                    &sorted_tracks,
                    header_height,
                    y_off,
                    bounds.height,
                );
            }

            // Draw MIDI clips
            for clip in self.midi_clips {
                self.draw_midi_clip(
                    frame,
                    clip,
                    &sorted_tracks,
                    header_height,
                    y_off,
                    bounds.height,
                );
            }

            // Lane-area portion of the loop in/out markers — the dim
            // overlays. The vertical loop lines, amber range fill, and
            // triangle handles are drawn below in the unclipped pass so
            // they cross over the ruler.
            if self.loop_enabled {
                let loop_in_x = self.sample_to_x(self.loop_in);
                let loop_out_x = self.sample_to_x(self.loop_out);

                if loop_in_x > 0.0 {
                    frame.fill_rectangle(
                        Point::new(0.0, header_height),
                        Size::new(loop_in_x.min(bounds.width), loop_dim_height),
                        loop_dim_color,
                    );
                }
                if loop_out_x < bounds.width {
                    let right_start = loop_out_x.max(0.0);
                    frame.fill_rectangle(
                        Point::new(right_start, header_height),
                        Size::new((bounds.width - right_start).max(0.0), loop_dim_height),
                        loop_dim_color,
                    );
                }
            }
        });

        // Draw bar/beat ruler (after the clipped lane pass so the ruler
        // labels always sit on top of the fixed-header backdrop).
        self.draw_ruler(frame, bounds.width, ruler_height);

        // Draw the unclipped portions of the loop markers — the ruler
        // amber fill, the vertical loop lines, and the triangle handles.
        // These intentionally cross the ruler / section band so the
        // handles read as draggable from above the lanes.
        if self.loop_enabled {
            let loop_in_x = self.sample_to_x(self.loop_in);
            let loop_out_x = self.sample_to_x(self.loop_out);
            let total_height = (header_height + track_area_height - y_off).max(bounds.height);
            let loop_color = theme::WARM;

            // Amber range fill in ruler area
            let range_x = loop_in_x.max(0.0);
            let range_w = (loop_out_x - range_x).max(0.0).min(bounds.width - range_x);
            if range_w > 0.0 {
                frame.fill_rectangle(
                    Point::new(range_x, 0.0),
                    Size::new(range_w, ruler_height),
                    Color::from_rgba(0.9, 0.72, 0.1, 0.15),
                );
            }

            // Loop In line + handle
            if loop_in_x >= -1.0 && loop_in_x <= bounds.width + 1.0 {
                frame.fill_rectangle(
                    Point::new(loop_in_x - 0.5, 0.0),
                    Size::new(1.0, total_height),
                    loop_color,
                );
                let tri = canvas::Path::new(|b| {
                    b.move_to(Point::new(loop_in_x - 6.0, 0.0));
                    b.line_to(Point::new(loop_in_x + 6.0, 0.0));
                    b.line_to(Point::new(loop_in_x, 8.0));
                    b.close();
                });
                frame.fill(&tri, loop_color);
            }

            // Loop Out line + handle
            if loop_out_x >= -1.0 && loop_out_x <= bounds.width + 1.0 {
                frame.fill_rectangle(
                    Point::new(loop_out_x - 0.5, 0.0),
                    Size::new(1.0, total_height),
                    loop_color,
                );
                let tri = canvas::Path::new(|b| {
                    b.move_to(Point::new(loop_out_x - 6.0, 0.0));
                    b.line_to(Point::new(loop_out_x + 6.0, 0.0));
                    b.line_to(Point::new(loop_out_x, 8.0));
                    b.close();
                });
                frame.fill(&tri, loop_color);
            }
        }

        // Playhead is drawn in the uncached overlay pass — see
        // `draw_overlay_into`. Keeping it out of the cached path lets
        // the rest of the timeline geometry stay cached during playback.

        // Smart scrollbars — drawn last so they sit above clips + playhead.
        let (h_rects, v_rects) = self.scrollbar_rects(bounds);

        let track_color = Color::from_rgba(0.08, 0.08, 0.08, 0.8);
        let thumb_color = Color::from_rgba(0.45, 0.45, 0.45, 0.85);

        if let Some(sb) = h_rects {
            frame.fill_rectangle(
                Point::new(sb.track.x, sb.track.y),
                Size::new(sb.track.width, sb.track.height),
                track_color,
            );
            frame.fill_rectangle(
                Point::new(sb.thumb.x, sb.thumb.y),
                Size::new(sb.thumb.width, sb.thumb.height),
                thumb_color,
            );
        }
        if let Some(sb) = v_rects {
            frame.fill_rectangle(
                Point::new(sb.track.x, sb.track.y),
                Size::new(sb.track.width, sb.track.height),
                track_color,
            );
            frame.fill_rectangle(
                Point::new(sb.thumb.x, sb.thumb.y),
                Size::new(sb.thumb.width, sb.thumb.height),
                thumb_color,
            );
        }

    }

    /// Draw the parts of the timeline that change every frame during
    /// playback / recording: the playhead line + tab, and the per-track
    /// recording overlay. Called from `Program::draw` on a fresh
    /// uncached `Frame` so these don't trigger cache invalidation.
    fn draw_overlay_into(&self, frame: &mut canvas::Frame, bounds: Rectangle) {
        let ruler_height = theme::RULER_HEIGHT;
        let header_height = self.fixed_header_height();
        let y_off = self.scroll_offset_y;
        let sorted_tracks = self.visible_tracks_sorted();
        let track_area_height = sorted_tracks.len() as f32 * theme::TRACK_HEIGHT;

        // Per-track recording overlay (one shaded strip per armed
        // track that spans from record-start to playhead, wrapping
        // around loop bounds when looping is active). Same clipping
        // discipline as `draw_into`: the strip is clipped to the lane
        // area so a partially-scrolled top row doesn't bleed its red
        // wash into the ruler / section band / global tracks header.
        if !self.recording_tracks.is_empty() {
            let lane_clip = Rectangle {
                x: 0.0,
                y: header_height,
                width: bounds.width,
                height: (bounds.height - header_height).max(0.0),
            };
            frame.with_clip(lane_clip, |frame| {
                for (i, track) in sorted_tracks.iter().enumerate() {
                    if !self.recording_tracks.contains(&track.id) {
                        continue;
                    }
                    let y = header_height + i as f32 * theme::TRACK_HEIGHT - y_off;
                    if y + theme::TRACK_HEIGHT < header_height || y > bounds.height {
                        continue;
                    }
                    let (overlay_start, overlay_end) = if self.loop_enabled {
                        (self.loop_in, self.playhead.min(self.loop_out))
                    } else {
                        (self.recording_start_sample, self.playhead)
                    };
                    let start_x = self.sample_to_x(overlay_start);
                    let end_x = self.sample_to_x(overlay_end);
                    let overlay_x = start_x.max(0.0);
                    let overlay_w =
                        (end_x - overlay_x).max(0.0).min(bounds.width - overlay_x);
                    if overlay_w > 0.0 {
                        frame.fill_rectangle(
                            Point::new(overlay_x, y),
                            Size::new(overlay_w, theme::TRACK_HEIGHT),
                            Color::from_rgba(0.8, 0.2, 0.2, 0.08),
                        );
                    }
                }
            });
        }

        // Playhead — warm 1px line + a rounded tab at the top.
        let playhead_seconds = (self.playhead as f64 / self.sample_rate as f64) as f32;
        let playhead_x = playhead_seconds * self.zoom - self.scroll_offset;
        if playhead_x >= 0.0 && playhead_x <= bounds.width {
            let total_height = (header_height + track_area_height - y_off).max(bounds.height);
            frame.fill_rectangle(
                Point::new(playhead_x - 0.5, 0.0),
                Size::new(1.0, total_height),
                theme::WARM,
            );
            let tab_w = 11.0;
            let tab_h = 11.0;
            let tab = canvas::Path::rounded_rectangle(
                Point::new(playhead_x - tab_w / 2.0, 0.0),
                Size::new(tab_w, tab_h),
                iced::border::radius(0.0).bottom(6.0),
            );
            frame.fill(&tab, theme::WARM);
        }
        // The ruler-height local is unused if neither overlay fires;
        // keep it so future overlay additions (e.g. selection brushes)
        // can use it without reintroducing the variable.
        let _ = ruler_height;
    }
}
