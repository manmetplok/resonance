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
//!
//! The canvas's concerns are split across files:
//!
//! - this file: [`VocalLaneCanvas`] struct, the `view` entry function,
//!   small free helpers, and the [`canvas::Program`] impl that
//!   orchestrates per-event dispatch and per-frame drawing.
//! - [`draw`]: pure-draw helpers (lyric band, bar lines, staff, real
//!   notes / synthesised contour).

use iced::widget::canvas::{self, Frame, Geometry};
use iced::widget::{container, Canvas};
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Theme};

use resonance_audio::types::{TempoMap, TrackId, TrackType};
use resonance_music_theory::{VocalContour, VocalParams};

use resonance_audio::types::ClipId;

use std::time::Instant;

use crate::compose::{
    ComposeMessage, LaneGeneratorKind, SectionDefinitionState, SectionPlacementState, SelectedLane,
};
use crate::message::{Message, MidiEditorMessage};
use crate::state::{MidiClipState, TrackState};
use crate::theme;
use crate::Resonance;

mod draw;

/// Maximum gap between two clicks for them to count as a double-click.
const DOUBLE_CLICK_MS: u128 = 350;

/// Height of a single vocal lane row. Slightly taller than the synth
/// lanes because the lyric flow needs a comfortable reading line.
pub const VOCAL_LANE_HEIGHT: f32 = 108.0;
/// Vertical split: top portion shows lyrics, bottom shows the contour.
pub(super) const LYRIC_BAND_HEIGHT: f32 = 42.0;
/// Compact height for the placeholder row shown for vocal tracks that
/// have no `Vocal` lane generator in this section yet. 64px is the
/// height `lane_side::draw`'s pill/title stack is calibrated for, and
/// the shorter row visually signals "nothing configured here".
pub(super) const PLACEHOLDER_ROW_HEIGHT: f32 = 64.0;

/// Build the vocal-lane stack. Returns an empty 0-height element when
/// the project has no vocal tracks at all.
///
/// Tracks with `TrackType::Vocal` that don't have a `Vocal` lane
/// generator configured for this section render as a compact,
/// selectable placeholder row instead of a full lyric/contour row —
/// fabricating default `VocalParams` (the pre-40300a4 behaviour) gave
/// the user visually wrong ranges (e.g. an alto staff for a soprano
/// track that hadn't been wired up yet), but skipping the track
/// entirely (the 40300a4 behaviour) made it unreachable: with no lane
/// to click there was no way to fire `SelectLane`, and the right-rail
/// generator picker — the only place a vocal generator can be
/// assigned — never opened. The placeholder restores selectability
/// without inventing params.
pub fn view<'a>(
    app: &'a Resonance,
    placement: &'a SectionPlacementState,
    definition: &'a SectionDefinitionState,
) -> Element<'a, Message> {
    let mut vocal_tracks: Vec<(TrackId, &VocalParams)> = Vec::new();
    let mut unconfigured: Vec<TrackId> = Vec::new();
    for t in app
        .registry
        .tracks
        .iter()
        .filter(|t| t.track_type == TrackType::Vocal)
    {
        match definition.lane_generators.get(&t.id).map(|cfg| &cfg.kind) {
            Some(LaneGeneratorKind::Vocal(p)) => vocal_tracks.push((t.id, p)),
            _ => unconfigured.push(t.id),
        }
    }

    if vocal_tracks.is_empty() && unconfigured.is_empty() {
        return container(iced::widget::Space::new().height(0))
            .width(Length::Fill)
            .into();
    }

    let total_height = vocal_tracks.len() as f32 * VOCAL_LANE_HEIGHT
        + unconfigured.len() as f32 * PLACEHOLDER_ROW_HEIGHT;
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
        unconfigured,
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

pub(super) struct VocalLaneCanvas<'a> {
    pub(super) tracks: &'a [TrackState],
    pub(super) vocal_tracks: Vec<(TrackId, &'a VocalParams)>,
    /// Vocal tracks with no `Vocal` lane generator in this section.
    /// Rendered as compact placeholder rows below the configured rows
    /// so they stay selectable (see the module-level `view` docs).
    pub(super) unconfigured: Vec<TrackId>,
    pub(super) tempo_map: &'a TempoMap,
    pub(super) start_bar: u32,
    pub(super) length_bars: u32,
    pub(super) selected_lane: SelectedLane,
    /// MIDI clips on every track. Used to draw the generated vocal
    /// melody on the staff once `derive_vocal` has produced notes.
    pub(super) midi_clips: &'a [MidiClipState],
    /// Authoritative `(track_id → derived_clip_id)` mapping for this
    /// placement, taken from `compose.derived_clips`. Lets the canvas
    /// find the generator-produced clip without guessing by
    /// `start_sample` (which can collide with a manually placed clip).
    pub(super) derived_clip_ids: std::collections::HashMap<u64, ClipId>,
}

/// Local canvas state — tracks the last single-click on a vocal lane
/// row so a second click within the double-click window can open the
/// vocal roll editor.
#[derive(Debug, Default)]
pub struct VocalLaneCanvasState {
    last_click: Option<(Instant, TrackId)>,
}

impl<'a> canvas::Program<Message> for VocalLaneCanvas<'a> {
    type State = VocalLaneCanvasState;

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::BG);

        if bounds.width <= 0.0
            || (self.vocal_tracks.is_empty() && self.unconfigured.is_empty())
        {
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

        // Placeholder rows for unconfigured vocal tracks, stacked below
        // the configured rows. Hover feedback comes from the cursor
        // position — this canvas has no `canvas::Cache`, so the fill
        // tracks the cursor without staleness.
        let configured_h = self.vocal_tracks.len() as f32 * VOCAL_LANE_HEIGHT;
        let cursor_pos = cursor.position_in(bounds);
        for (idx, track_id) in self.unconfigured.iter().enumerate() {
            let row_rect = Rectangle {
                x: 0.0,
                y: configured_h + idx as f32 * PLACEHOLDER_ROW_HEIGHT,
                width: bounds.width,
                height: PLACEHOLDER_ROW_HEIGHT,
            };
            let hovered = cursor_pos.is_some_and(|p| row_rect.contains(p));
            self.draw_placeholder_row(&mut frame, *track_id, row_rect, hovered);
        }

        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if let iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            let pos = cursor.position_in(bounds)?;

            // Clicks below the configured rows land on a placeholder row
            // (unconfigured vocal track). There is no derived clip and no
            // params there — the only meaningful action is selecting the
            // lane so the right-rail generator picker opens.
            let configured_h = self.vocal_tracks.len() as f32 * VOCAL_LANE_HEIGHT;
            if pos.y >= configured_h {
                let idx = ((pos.y - configured_h) / PLACEHOLDER_ROW_HEIGHT) as usize;
                let track_id = self.unconfigured.get(idx)?;
                return Some(canvas::Action::publish(Message::Compose(
                    ComposeMessage::SelectLane(SelectedLane::Instrument(*track_id)),
                ))
                .and_capture());
            }

            let idx = (pos.y / VOCAL_LANE_HEIGHT) as usize;
            let (track_id, _) = self.vocal_tracks.get(idx)?;

            // Double-click anywhere on the row opens the derived MIDI
            // clip in the vocal roll editor. Fall back to the lane
            // selection message on single-click.
            let now = Instant::now();
            let is_double_click = state
                .last_click
                .map(|(t, tid)| {
                    tid == *track_id && now.duration_since(t).as_millis() <= DOUBLE_CLICK_MS
                })
                .unwrap_or(false);
            state.last_click = Some((now, *track_id));
            if is_double_click {
                state.last_click = None;
                if let Some(clip_id) = self.derived_clip_ids.get(track_id).copied() {
                    return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::OpenMidiEditor(
                            clip_id,
                        ))).and_capture());
                }
                // Fallback: even without a derived clip, still focus the
                // lane so the user sees the right rail. Avoids a dead
                // double-click before the first regenerate.
            }
            return Some(canvas::Action::publish(Message::Compose(ComposeMessage::SelectLane(
                    SelectedLane::Instrument(*track_id),
                ))).and_capture());
        }
        None
    }
}

/// Contour value 0.0 (low) .. 1.0 (high) at normalised time `t` ∈ [0, 1].
/// Used to synthesise a visual melody until the real generator lands.
pub(super) fn contour_value(contour: VocalContour, t: f32) -> f32 {
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
