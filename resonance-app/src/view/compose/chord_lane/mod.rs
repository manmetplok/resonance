//! Chord lane visualisation for the Compose workspace.
//!
//! The canvas's concerns are split across files:
//!
//! - this file: [`ChordLaneCanvas`] struct, the `view` entry function,
//!   small free helpers, and the [`canvas::Program`] impl that
//!   orchestrates per-event dispatch and per-frame drawing.
//! - [`draw`]: pure-draw helpers ([`ChordLaneCanvas::draw_into`]).
//! - [`input`]: pointer interaction handlers ([`ChordLaneCanvas::update_inner`]).

use iced::widget::canvas::{self, Frame, Geometry};
use iced::widget::Canvas;
use iced::{mouse, Element, Length, Rectangle, Renderer, Theme};

use resonance_audio::types::TempoMap;

use crate::compose::SectionDefinitionState;
use crate::message::Message;

mod draw;
mod input;

pub const LANE_HEIGHT: f32 = 64.0;
pub(super) const RULER_HEIGHT: f32 = 18.0;
pub(super) const DEFAULT_NEW_CHORD_BEATS: u32 = 4;
/// Horizontal pixels from a chord block's right edge that count as the
/// resize handle. Anything inside that strip starts a resize drag; the rest
/// of the body starts a move drag.
pub(super) const RESIZE_HANDLE_PX: f32 = 8.0;

pub fn view<'a>(
    definition: &'a SectionDefinitionState,
    tempo_map: &'a TempoMap,
    start_bar: u32,
    selected_chord_id: Option<u64>,
    chords_selected: bool,
) -> Element<'a, Message> {
    let width = super::workspace_width(tempo_map, start_bar, definition.length_bars);
    Canvas::new(ChordLaneCanvas {
        definition,
        tempo_map,
        start_bar,
        selected_chord_id,
        chords_selected,
    })
    .width(Length::Fixed(width))
    .height(Length::Fixed(LANE_HEIGHT))
    .into()
}

pub struct ChordLaneCanvas<'a> {
    pub definition: &'a SectionDefinitionState,
    pub tempo_map: &'a TempoMap,
    pub start_bar: u32,
    pub selected_chord_id: Option<u64>,
    pub chords_selected: bool,
}

#[derive(Debug, Default)]
pub struct ChordLaneState {
    pub(super) drag: Option<ChordDrag>,
    /// Geometry cache. Repaints only when the section data, drag state,
    /// or chord selection changes — hover events on sibling widgets
    /// reuse the stored geometry.
    cache: canvas::Cache,
    cache_fingerprint: std::cell::Cell<ChordLaneFingerprint>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ChordLaneFingerprint {
    chord_count: usize,
    chord_layout_hash: u64,
    selected_chord_id: Option<u64>,
    chords_selected: bool,
    drag_active: bool,
    start_bar: u32,
    length_bars: u32,
    tempo_points: usize,
    sig_points: usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ChordDrag {
    /// Moving a chord: `grab_beat` is the beat offset inside the chord where
    /// the mouse grabbed it, so the chord sticks to the cursor naturally.
    Move {
        chord_id: u64,
        grab_beat: u32,
        pending_start_beat: u32,
    },
    /// Resizing a chord from its right edge.
    Resize {
        chord_id: u64,
        pending_duration_beats: u32,
    },
}

impl<'a> canvas::Program<Message> for ChordLaneCanvas<'a> {
    type State = ChordLaneState;

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let drag_active = state.drag.is_some();
        let layout_hash = chord_layout_hash(self.definition);
        let fp = ChordLaneFingerprint {
            chord_count: self.definition.chords.len(),
            chord_layout_hash: layout_hash,
            selected_chord_id: self.selected_chord_id,
            chords_selected: self.chords_selected,
            drag_active,
            start_bar: self.start_bar,
            length_bars: self.definition.length_bars,
            tempo_points: self.tempo_map.tempo_points.len(),
            sig_points: self.tempo_map.signature_points.len(),
        };
        if state.cache_fingerprint.get() != fp {
            state.cache.clear();
            state.cache_fingerprint.set(fp);
        }
        let geometry = state
            .cache
            .draw(renderer, bounds.size(), |frame: &mut Frame| {
                self.draw_into(frame, bounds, &state.drag);
            });
        vec![geometry]
    }

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        self.update_inner(state, event, bounds, cursor)
    }
}

fn chord_layout_hash(def: &SectionDefinitionState) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for c in &def.chords {
        c.id.hash(&mut h);
        c.start_beat.hash(&mut h);
        c.duration_beats.hash(&mut h);
        format!("{}", c.chord).hash(&mut h);
    }
    // Scale feeds the side panel's meta line ("B · 5 chords") so changes to
    // it have to invalidate the cache.
    if let Some(scale) = def.scale.as_ref() {
        scale.root.to_semitone().hash(&mut h);
        (scale.mode as u8).hash(&mut h);
    } else {
        0u8.hash(&mut h);
    }
    h.finish()
}

impl<'a> ChordLaneCanvas<'a> {
    /// Total beats in the section, summing per-bar numerators.
    pub(super) fn total_beats(&self) -> u32 {
        (0..self.definition.length_bars)
            .map(|b| self.tempo_map.numerator_at_bar(self.start_bar + b) as u32)
            .sum()
    }
}

/// Roman-numeral degree label for a chord against its section's scale.
/// Lowercase numerals for minor / diminished qualities; "—" when the
/// scale is unknown or the chord root isn't a diatonic degree.
pub(super) fn roman_numeral_for(
    chord: &resonance_music_theory::Chord,
    scale: Option<&resonance_music_theory::Scale>,
) -> String {
    use resonance_music_theory::ChordQuality;
    let Some(scale) = scale else {
        return String::from("—");
    };
    let root_semi = chord.root.to_semitone() as i32;
    let scale_root = scale.root.to_semitone() as i32;
    let interval = ((root_semi - scale_root) + 12) % 12;
    let mut degree_idx = None;
    for (i, &iv) in scale.mode.intervals().iter().enumerate() {
        if iv as i32 == interval {
            degree_idx = Some(i);
            break;
        }
    }
    let Some(idx) = degree_idx else {
        return String::from("—");
    };
    let upper = ["I", "II", "III", "IV", "V", "VI", "VII"];
    let lower = ["i", "ii", "iii", "iv", "v", "vi", "vii"];
    let is_minor_like = matches!(
        chord.quality,
        ChordQuality::Min
            | ChordQuality::Min7
            | ChordQuality::Dim
            | ChordQuality::Dim7
            | ChordQuality::HalfDim7
    );
    let mut s = if is_minor_like {
        lower[idx].to_string()
    } else {
        upper[idx].to_string()
    };
    if matches!(
        chord.quality,
        ChordQuality::Dim | ChordQuality::Dim7 | ChordQuality::HalfDim7
    ) {
        s.push('°');
    }
    s
}

/// Applies the active drag's pending values for the given chord so the
/// draw pass can render the preview in place of the persisted state.
pub(super) fn apply_drag_preview(
    chord_id: u64,
    start_beat: u32,
    duration_beats: u32,
    drag: &Option<ChordDrag>,
) -> (u32, u32) {
    match drag {
        Some(ChordDrag::Move {
            chord_id: id,
            pending_start_beat,
            ..
        }) if *id == chord_id => (*pending_start_beat, duration_beats),
        Some(ChordDrag::Resize {
            chord_id: id,
            pending_duration_beats,
        }) if *id == chord_id => (start_beat, *pending_duration_beats),
        _ => (start_beat, duration_beats),
    }
}
