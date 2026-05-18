//! Cropped, inline-editable track canvas for the Compose tab. Only
//! instrument tracks are rendered. Each row shows a mini piano-roll grid
//! spanning PITCH_RANGE_LOW..=PITCH_RANGE_HIGH with the clip's notes drawn
//! as colored blocks. Click an empty cell to add a note, click an existing
//! note to remove it, click the "+" hint to spawn a clip that spans the
//! whole section, or click the name column on the left to open the
//! instrument details panel on the right side of the Compose tab.
//!
//! The canvas's concerns are split across files:
//!
//! - this file: [`ComposeTrackCanvas`] struct, the `view` entry function,
//!   small helpers, and shared constants.
//! - [`canvas`]: the [`canvas::Program`] impl that orchestrates per-event
//!   dispatch and per-frame drawing.
//! - [`draw`]: pure-draw helpers and hit-test methods on
//!   [`ComposeTrackCanvas`].

use std::time::Instant;

use iced::widget::{container, Canvas};
use iced::{Element, Length};

use resonance_audio::types::{TempoMap, TrackId, TrackType, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::Scale;

use crate::compose::{SectionDefinitionState, SectionPlacementState};
use crate::message::*;
use crate::state::{InstrumentType, MidiClipState, TrackState};
use crate::Resonance;

use super::lane_side::LaneKind;

mod canvas;
mod draw;

/// Heuristic: lead-named instrument tracks read as "MELODY", everything else
/// (bass, pad, generic synth) reads as "ACCOMP". Mirrors the design's
/// per-lane tag treatment.
pub(super) fn lane_kind_for(track: &TrackState) -> LaneKind {
    if track.instrument_type == InstrumentType::Drum {
        LaneKind::Rhythm
    } else if track.name.to_ascii_lowercase().contains("lead") {
        LaneKind::Melody
    } else {
        LaneKind::Accomp
    }
}

/// Meta line shown under the track's name in the side panel. Falls back to
/// the instrument type label when no plugin slot is populated.
pub(super) fn track_meta_line(track: &TrackState) -> String {
    if let Some(slot) = track.plugins.first() {
        if !slot.plugin_name.is_empty() {
            return slot.plugin_name.clone();
        }
    }
    track.instrument_type.as_str().to_string()
}

/// Width reserved on the left edge of the canvas for an inline track-name
/// label. The Compose tab intentionally does not show the full track header
/// (mute/solo/arm/plugins) — that is Arrange-only territory. Clicking the
/// name column opens the instrument details view in the right-side panel.
pub const NAME_COLUMN_WIDTH: f32 = 168.0;
/// Row height used inside the Compose track area. Matches the bundled
/// design's lane height closely while still giving the inline piano grid
/// enough room to read pitch contour at a glance — fine-grained editing
/// happens in the expanded editor / piano-roll overlay.
pub(super) const COMPOSE_TRACK_HEIGHT: f32 = 120.0;
/// Height for collapsed track strips when another track is expanded.
pub(super) const COLLAPSED_TRACK_HEIGHT: f32 = 36.0;
/// Top/bottom padding inside each row before the note grid starts.
pub(super) const NOTE_GRID_PAD: f32 = 6.0;
/// Pitch range shown inline. C2 .. C6 covers most melodic writing and keeps
/// semitone cells tall enough to click reliably.
pub(super) const PITCH_RANGE_LOW: u8 = 36; // C2
pub(super) const PITCH_RANGE_HIGH: u8 = 84; // C6
pub(super) const DEFAULT_NEW_NOTE_TICKS: u64 = TICKS_PER_QUARTER_NOTE;
pub(super) const DEFAULT_NEW_NOTE_VELOCITY: f32 = 0.8;
/// Size of the "+" hint button drawn over empty instrument rows.
pub(super) const ADD_BUTTON_SIZE: f32 = 32.0;
/// Maximum milliseconds between two clicks to count as a double-click.
pub(super) const DOUBLE_CLICK_MS: u64 = 400;

pub(super) fn pitch_count() -> u8 {
    PITCH_RANGE_HIGH - PITCH_RANGE_LOW + 1
}

pub fn view<'a>(
    app: &'a Resonance,
    placement: &'a SectionPlacementState,
    definition: &'a SectionDefinitionState,
) -> Element<'a, Message> {
    let section_start = app.tempo_map.bar_to_sample(placement.start_bar);
    let section_end = app.tempo_map.bar_to_sample(placement.start_bar + definition.length_bars);

    // Vocal tracks render in their own lyric/contour lane above the synth
    // canvas, so they're filtered out here via `TrackType::Vocal`. Drum
    // tracks live in the drumroll canvas, and sub-tracks are driven from
    // their parent.
    let instr_count = app
        .registry
        .tracks
        .iter()
        .filter(|t| {
            matches!(t.track_type, resonance_audio::types::TrackType::Instrument)
                && t.sub_track.is_none()
                && t.instrument_type != InstrumentType::Drum
        })
        .count() as f32;
    let total_height = if app.compose.expanded_track_id.is_some() {
        instr_count.max(1.0) * COLLAPSED_TRACK_HEIGHT
    } else {
        instr_count.max(1.0) * COMPOSE_TRACK_HEIGHT
    };

    let width = super::workspace_width(
        &app.tempo_map,
        placement.start_bar,
        definition.length_bars,
    );
    let cropped = Canvas::new(ComposeTrackCanvas {
        tracks: &app.registry.tracks,
        midi_clips: &app.midi_clips,
        section_start,
        section_end,
        section_length_bars: definition.length_bars,
        sample_rate: app.sample_rate,
        tempo_map: &app.tempo_map,
        start_bar: placement.start_bar,
        scroll_offset_y: app.viewport.scroll_offset_y,
        scale: definition.scale,
        details_track_id: app.compose.details_track_id(),
        expanded_track_id: app.compose.expanded_track_id,
    })
    .width(Length::Fixed(width))
    .height(Length::Fixed(total_height));

    container(cropped)
        .width(Length::Fixed(width))
        .height(Length::Fixed(total_height))
        .into()
}

/// Cropped, inline-editable track canvas for the Compose tab.
pub struct ComposeTrackCanvas<'a> {
    pub tracks: &'a [TrackState],
    pub midi_clips: &'a [MidiClipState],
    pub section_start: u64,
    pub section_end: u64,
    pub section_length_bars: u32,
    pub sample_rate: u32,
    pub tempo_map: &'a TempoMap,
    pub start_bar: u32,
    pub scroll_offset_y: f32,
    pub scale: Option<Scale>,
    pub details_track_id: Option<TrackId>,
    /// When set, this track is expanded into the full editor; other tracks
    /// are rendered as collapsed name-only strips.
    pub expanded_track_id: Option<TrackId>,
}

/// Canvas-local state for double-click detection.
#[derive(Debug, Default)]
pub struct ComposeTrackCanvasState {
    pub(super) last_click: Option<(Instant, TrackId)>,
}

impl<'a> ComposeTrackCanvas<'a> {
    pub(super) fn sorted_tracks(&self) -> Vec<&TrackState> {
        // Exclude sub-tracks: they don't accept MIDI (their audio comes
        // from their parent plugin's output port) and would clutter the
        // Compose instrument list with empty rows. Vocal tracks render in
        // a dedicated lyric/contour lane above this canvas.
        let mut v: Vec<&TrackState> = self
            .tracks
            .iter()
            .filter(|t| {
                matches!(t.track_type, TrackType::Instrument)
                    && t.sub_track.is_none()
                    && t.instrument_type != InstrumentType::Drum
            })
            .collect();
        v.sort_by_key(|t| t.order);
        v
    }
}

pub(super) fn snap_tick(tick: u64, snap: u64) -> u64 {
    if snap == 0 {
        return tick;
    }
    (tick / snap) * snap
}
