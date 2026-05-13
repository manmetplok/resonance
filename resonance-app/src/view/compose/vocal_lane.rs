//! Vocal lane visualisation for the Compose workspace.
//!
//! Renders one full-width row per track in the current section that has
//! a `LaneGeneratorKind::Vocal` configuration. The row shows:
//!
//! * **Header (`NAME_COLUMN_WIDTH`)** — track name and "Vocal · {voice}"
//!   meta line, matching the other lane-side panels.
//! * **Lyric flow (top half)** — the draft's first 1–2 lines as italic
//!   serif text, syllables separated by typographic `·` dots.
//! * **Melody contour (bottom half)** — a 5-line staff with warm-coloured
//!   note bars positioned by pitch. Notes derive from the params'
//!   `contour` shape so the visual changes when the user changes the
//!   contour chip in the right rail.
//!
//! The canvas uses a fixed pixel width matching the other Compose lanes
//! (`workspace_width`) so the lyrics and the synth/drum grids stay
//! aligned and don't stretch when the OS window resizes.

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{container, Canvas};
use iced::{mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use resonance_audio::types::{TempoMap, TrackId, TrackType};
use resonance_music_theory::{VocalContour, VocalParams};

use resonance_audio::types::ClipId;

use crate::compose::{
    ComposeMessage, LaneGeneratorKind, SectionDefinitionState, SectionPlacementState, SelectedLane,
};
use crate::message::Message;
use crate::state::{MidiClipState, TrackState};
use crate::theme;
use crate::Resonance;

use super::lane_side::{self, LaneKind};
use super::tracks::NAME_COLUMN_WIDTH;

/// Height of a single vocal lane row. Slightly taller than the synth
/// lanes because the lyric flow needs a comfortable reading line.
pub const VOCAL_LANE_HEIGHT: f32 = 108.0;
/// Vertical split: top portion shows lyrics, bottom shows the contour.
const LYRIC_BAND_HEIGHT: f32 = 42.0;

/// Build the vocal-lane stack. Returns an empty 0-height element when
/// the section has no vocal-generator lanes.
/// Fallback when a vocal track exists but the section hasn't been wired
/// to a `LaneGeneratorKind::Vocal` yet. Lets the lane still render
/// (with default contour + lyrics) instead of disappearing.
fn default_vocal_params() -> &'static VocalParams {
    use std::sync::OnceLock;
    static DEFAULT: OnceLock<VocalParams> = OnceLock::new();
    DEFAULT.get_or_init(VocalParams::default)
}

pub fn view<'a>(
    app: &'a Resonance,
    placement: &'a SectionPlacementState,
    definition: &'a SectionDefinitionState,
) -> Element<'a, Message> {
    // Vocal tracks are identified by their track type (engine-level
    // attribute), not by a per-section lane-generator configuration.
    // Lane params still live on the section's `LaneGeneratorKind::Vocal`;
    // tracks without a configured generator fall back to defaults so the
    // lane is always drawable.
    let vocal_tracks: Vec<(TrackId, &VocalParams)> = app
        .registry
        .tracks
        .iter()
        .filter(|t| t.track_type == TrackType::Vocal)
        .map(|t| {
            let params = definition
                .lane_generators
                .get(&t.id)
                .and_then(|cfg| match &cfg.kind {
                    LaneGeneratorKind::Vocal(p) => Some(p),
                    _ => None,
                })
                .unwrap_or_else(|| default_vocal_params());
            (t.id, params)
        })
        .collect();

    if vocal_tracks.is_empty() {
        return container(iced::widget::Space::with_height(0))
            .width(Length::Fill)
            .into();
    }

    let total_height = vocal_tracks.len() as f32 * VOCAL_LANE_HEIGHT;
    let width = super::workspace_width(
        &app.tempo_map,
        placement.start_bar,
        definition.length_bars,
    );

    // Snapshot the (track_id → derived_clip_id) mapping for this
    // placement. The vocal canvas uses it to find the generated MIDI
    // clip authoritatively, instead of guessing by `start_sample`
    // which can collide with a user-placed clip at bar 0.
    let derived_clip_ids: std::collections::HashMap<u64, ClipId> = app
        .compose
        .derived_clips
        .iter()
        .filter(|((def_id, plac_id, _), _)| {
            *def_id == definition.id && *plac_id == placement.id
        })
        .map(|((_, _, tid), cid)| (*tid, *cid))
        .collect();

    let canvas_prog = VocalLaneCanvas {
        tracks: &app.registry.tracks,
        vocal_tracks,
        tempo_map: &app.tempo_map,
        start_bar: placement.start_bar,
        length_bars: definition.length_bars,
        selected_lane: app.compose.selected_lane.clone(),
        midi_clips: &app.midi_clips,
        derived_clip_ids,
    };

    container(
        Canvas::new(canvas_prog)
            .width(Length::Fixed(width))
            .height(Length::Fixed(total_height)),
    )
    .width(Length::Fixed(width))
    .height(Length::Fixed(total_height))
    .into()
}

struct VocalLaneCanvas<'a> {
    tracks: &'a [TrackState],
    vocal_tracks: Vec<(TrackId, &'a VocalParams)>,
    tempo_map: &'a TempoMap,
    start_bar: u32,
    length_bars: u32,
    selected_lane: SelectedLane,
    /// MIDI clips on every track. Used to draw the generated vocal
    /// melody on the staff once `derive_vocal` has produced notes.
    midi_clips: &'a [MidiClipState],
    /// Authoritative `(track_id → derived_clip_id)` mapping for this
    /// placement, taken from `compose.derived_clips`. Lets the canvas
    /// find the generator-produced clip without guessing by
    /// `start_sample` (which can collide with a manually placed clip).
    derived_clip_ids: std::collections::HashMap<u64, ClipId>,
}

impl<'a> canvas::Program<Message> for VocalLaneCanvas<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::BG);

        if bounds.width <= 0.0 || self.vocal_tracks.is_empty() {
            return vec![frame.into_geometry()];
        }

        for (idx, (track_id, params)) in self.vocal_tracks.iter().enumerate() {
            let y = idx as f32 * VOCAL_LANE_HEIGHT;
            let row_rect = Rectangle {
                x: 0.0,
                y,
                width: bounds.width,
                height: VOCAL_LANE_HEIGHT,
            };
            self.draw_row(&mut frame, *track_id, params, row_rect);
        }

        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        _state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        if let canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            let Some(pos) = cursor.position_in(bounds) else {
                return (canvas::event::Status::Ignored, None);
            };
            let idx = (pos.y / VOCAL_LANE_HEIGHT) as usize;
            if let Some((track_id, _)) = self.vocal_tracks.get(idx) {
                return (
                    canvas::event::Status::Captured,
                    Some(Message::Compose(ComposeMessage::SelectLane(
                        SelectedLane::Instrument(*track_id),
                    ))),
                );
            }
        }
        (canvas::event::Status::Ignored, None)
    }
}

impl<'a> VocalLaneCanvas<'a> {
    fn track_name(&self, track_id: TrackId) -> &str {
        self.tracks
            .iter()
            .find(|t| t.id == track_id)
            .map(|t| t.name.as_str())
            .unwrap_or("Vocal")
    }

    fn draw_row(
        &self,
        frame: &mut Frame,
        track_id: TrackId,
        params: &VocalParams,
        row_rect: Rectangle,
    ) {
        let is_selected = matches!(
            self.selected_lane,
            SelectedLane::Instrument(t) if t == track_id
        );

        // Side panel — name + meta line.
        let side_rect = Rectangle {
            x: row_rect.x,
            y: row_rect.y,
            width: NAME_COLUMN_WIDTH,
            height: row_rect.height,
        };
        let meta = format!("Vocal \u{00B7} {}", params.voice.as_str());
        lane_side::draw(
            frame,
            side_rect,
            LaneKind::Melody,
            self.track_name(track_id),
            Some(&meta),
            is_selected,
        );

        let lane_rect = Rectangle {
            x: row_rect.x + NAME_COLUMN_WIDTH,
            y: row_rect.y,
            width: (row_rect.width - NAME_COLUMN_WIDTH).max(0.0),
            height: row_rect.height,
        };

        // Background fill — warm tint if the lane is selected, BG_1 otherwise.
        let fill = if is_selected {
            Color {
                a: 0.10,
                ..theme::WARM
            }
        } else {
            theme::BG_1
        };
        frame.fill_rectangle(
            Point::new(lane_rect.x, lane_rect.y),
            Size::new(lane_rect.width, lane_rect.height),
            fill,
        );

        // Lyric band at the top
        self.draw_lyric_band(frame, lane_rect, params);

        // Bar lines across the lane for context
        self.draw_bar_lines(frame, lane_rect);

        // Melody contour (5-line staff) in the bottom band
        let staff_rect = Rectangle {
            x: lane_rect.x,
            y: lane_rect.y + LYRIC_BAND_HEIGHT,
            width: lane_rect.width,
            height: lane_rect.height - LYRIC_BAND_HEIGHT,
        };
        if !self.draw_real_notes(frame, staff_rect, track_id, params) {
            self.draw_melody_contour(frame, staff_rect, params);
        }

        // Bottom separator
        frame.fill_rectangle(
            Point::new(row_rect.x, row_rect.y + row_rect.height - 1.0),
            Size::new(row_rect.width, 1.0),
            theme::SEPARATOR,
        );
    }

    /// Top band: flowing italic lyric line. Pulls the first 1–2 unlocked
    /// or locked draft lines and concatenates them with a phrase-break
    /// slash.
    fn draw_lyric_band(&self, frame: &mut Frame, lane_rect: Rectangle, params: &VocalParams) {
        // Subtle band separator at the bottom of the lyric area.
        frame.fill_rectangle(
            Point::new(lane_rect.x, lane_rect.y + LYRIC_BAND_HEIGHT - 1.0),
            Size::new(lane_rect.width, 1.0),
            theme::LINE_2,
        );

        let text_y = lane_rect.y + LYRIC_BAND_HEIGHT * 0.5 - 9.0;

        if params.draft.is_empty() {
            frame.fill_text(canvas::Text {
                content: "(no lyrics yet \u{2014} hit Generate)".to_string(),
                position: Point::new(lane_rect.x + 12.0, text_y),
                color: theme::TEXT_3,
                size: 11.0.into(),
                font: theme::SERIF_ITALIC_FONT,
                ..canvas::Text::default()
            });
            return;
        }

        // Take the first two lines for the flow display and join them with
        // a phrase-break separator. Strip the syllable `·` markers — those
        // belong in the right-rail draft, not the timeline visualisation.
        let mut combined = String::new();
        for (i, line) in params.draft.iter().take(2).enumerate() {
            if i > 0 {
                combined.push_str("  /  ");
            }
            for ch in line.text.chars() {
                if ch == '\u{00B7}' {
                    combined.push(' ');
                } else {
                    combined.push(ch);
                }
            }
        }

        frame.fill_text(canvas::Text {
            content: combined,
            position: Point::new(lane_rect.x + 12.0, text_y),
            color: theme::TEXT_1,
            size: 16.0.into(),
            font: theme::SERIF_ITALIC_FONT,
            ..canvas::Text::default()
        });
    }

    /// Locate the generator-produced MIDI clip for `track_id` in this
    /// placement. Falls back to `None` when nothing has been generated
    /// yet — the caller paints the synthetic contour preview instead.
    fn vocal_clip(&self, track_id: TrackId) -> Option<&MidiClipState> {
        let clip_id = *self.derived_clip_ids.get(&track_id)?;
        self.midi_clips.iter().find(|c| c.id == clip_id)
    }

    /// Draw the staff with the actual generated MIDI notes on it. Returns
    /// `true` when notes were drawn; `false` when no clip exists yet
    /// (caller falls back to the synthesised contour).
    fn draw_real_notes(
        &self,
        frame: &mut Frame,
        staff_rect: Rectangle,
        track_id: TrackId,
        params: &VocalParams,
    ) -> bool {
        let Some(clip) = self.vocal_clip(track_id) else {
            return false;
        };
        if clip.notes.is_empty() {
            return false;
        }

        // Staff lines.
        let line_spacing = staff_rect.height / 6.0;
        let line_top = staff_rect.y + line_spacing;
        for i in 0..5 {
            let y = line_top + i as f32 * line_spacing;
            frame.fill_rectangle(
                Point::new(staff_rect.x + 8.0, y),
                Size::new(staff_rect.width - 16.0, 1.0),
                theme::LINE_2,
            );
        }

        let inner_x = staff_rect.x + 8.0;
        let inner_w = (staff_rect.width - 16.0).max(0.0);
        let staff_top = line_top - line_spacing * 0.5;
        let staff_bottom = line_top + 4.0 * line_spacing + line_spacing * 0.5;
        let staff_h = staff_bottom - staff_top;

        let (lo, hi) = params.range;
        let range_span = (hi.saturating_sub(lo)).max(1) as f32;
        let total_ticks = clip.duration_ticks.max(1) as f32;

        for note in &clip.notes {
            let t = note.start_tick as f32 / total_ticks;
            let dur_t = (note.duration_ticks as f32 / total_ticks).max(0.005);
            let pitch_norm = ((note.note.saturating_sub(lo)) as f32 / range_span).clamp(0.0, 1.0);
            let y = staff_bottom - pitch_norm * staff_h - 1.5;
            let x = inner_x + t * inner_w;
            let w = (dur_t * inner_w).max(3.0);
            let strong = note.velocity > 0.83;
            let color = if strong {
                theme::WARM
            } else {
                Color { a: 0.78, ..theme::WARM }
            };
            let path = Path::rounded_rectangle(
                Point::new(x, y),
                Size::new(w, 3.0),
                1.5.into(),
            );
            frame.fill(&path, color);
        }

        // Faint section playhead reference: 1px warm vertical at 12% of
        // section width. Visual cue only.
        let ph_x = inner_x + inner_w * 0.12;
        frame.stroke(
            &Path::line(
                Point::new(ph_x, staff_rect.y + 4.0),
                Point::new(ph_x, staff_rect.y + staff_rect.height - 4.0),
            ),
            Stroke::default().with_width(1.0).with_color(Color {
                a: 0.30,
                ..theme::WARM
            }),
        );
        true
    }

    /// 5-line staff with warm note bars positioned by a synthesised
    /// contour that follows `params.contour`. This is a visual stand-in
    /// for the future `derive_vocal` melody output.
    fn draw_melody_contour(&self, frame: &mut Frame, staff_rect: Rectangle, params: &VocalParams) {
        // 5 staff lines, vertically distributed across the staff_rect.
        let line_spacing = staff_rect.height / 6.0;
        let line_top = staff_rect.y + line_spacing;
        for i in 0..5 {
            let y = line_top + i as f32 * line_spacing;
            frame.fill_rectangle(
                Point::new(staff_rect.x + 8.0, y),
                Size::new(staff_rect.width - 16.0, 1.0),
                theme::LINE_2,
            );
        }

        // Synthesise contour points. Note count scales with the section's
        // length so wider sections show more notes without changing per-
        // note pixel width.
        let total_beats = super::section_total_beats(self.tempo_map, self.start_bar, self.length_bars);
        let note_count = (total_beats * 2).clamp(8, 64) as usize; // 2 notes per beat
        if note_count == 0 {
            return;
        }
        let inner_x = staff_rect.x + 8.0;
        let inner_w = (staff_rect.width - 16.0).max(0.0);
        let note_w = (inner_w / note_count as f32) * 0.62;
        let stride = inner_w / note_count as f32;
        let staff_top = line_top - line_spacing * 0.5;
        let staff_bottom = line_top + 4.0 * line_spacing + line_spacing * 0.5;
        let staff_h = staff_bottom - staff_top;

        for i in 0..note_count {
            let t = i as f32 / (note_count.saturating_sub(1).max(1) as f32);
            let pitch = contour_value(params.contour, t);
            // Map pitch (0..1) inverted (high pitch = top of staff).
            let y = staff_bottom - pitch * staff_h - 2.0;
            let x = inner_x + i as f32 * stride;
            let accent = (i % 8) == 0;
            let h = if accent { 5.0 } else { 3.0 };
            let color = if accent {
                theme::WARM
            } else {
                Color {
                    a: 0.75,
                    ..theme::WARM
                }
            };
            let path = Path::rounded_rectangle(
                Point::new(x, y - h * 0.5),
                Size::new(note_w.max(2.0), h),
                1.5.into(),
            );
            frame.fill(&path, color);
        }

        // Playhead suggestion (faint warm vertical at 12%).
        let ph_x = inner_x + inner_w * 0.12;
        frame.stroke(
            &Path::line(
                Point::new(ph_x, staff_rect.y + 4.0),
                Point::new(ph_x, staff_rect.y + staff_rect.height - 4.0),
            ),
            Stroke::default().with_width(1.0).with_color(Color {
                a: 0.35,
                ..theme::WARM
            }),
        );
    }

    fn draw_bar_lines(&self, frame: &mut Frame, lane_rect: Rectangle) {
        let total_beats =
            super::section_total_beats(self.tempo_map, self.start_bar, self.length_bars);
        if total_beats == 0 {
            return;
        }
        let beat_px = lane_rect.width / total_beats as f32;
        let mut beat_pos: u32 = 0;
        for bar_offset in 0..self.length_bars {
            let bar = self.start_bar + bar_offset;
            let num = self.tempo_map.numerator_at_bar(bar) as u32;
            if bar_offset > 0 {
                let x = lane_rect.x + beat_pos as f32 * beat_px;
                frame.fill_rectangle(
                    Point::new(x, lane_rect.y),
                    Size::new(1.0, lane_rect.height),
                    theme::LINE_2,
                );
            }
            beat_pos += num;
        }
    }
}

/// Contour value 0.0 (low) .. 1.0 (high) at normalised time `t` ∈ [0, 1].
/// Used to synthesise a visual melody until the real generator lands.
fn contour_value(contour: VocalContour, t: f32) -> f32 {
    use std::f32::consts::PI;
    match contour {
        // Arch: rise to mid-bar, fall back.
        VocalContour::Arch => (PI * t).sin().clamp(0.0, 1.0),
        // Rise: gentle ascent from 0.15 to 0.95.
        VocalContour::Rise => 0.15 + t * 0.80,
        // Fall: descent from 0.95 to 0.15.
        VocalContour::Fall => 0.95 - t * 0.80,
        // Wave: 1.5 cycle sin shifted to [0, 1].
        VocalContour::Wave => 0.5 + 0.4 * (1.5 * 2.0 * PI * t).sin(),
        // Flat: small noise around mid.
        VocalContour::Flat => 0.5 + 0.05 * (8.0 * t).sin(),
    }
}
