use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::Canvas;
use iced::{mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use resonance_audio::types::TempoMap;
use resonance_music_theory::{ChordQuality, PitchClass};

use crate::compose::{ComposeMessage, SelectedLane, SectionDefinitionState};
use crate::message::Message;
use crate::theme;

use super::lane_side::{self, LaneKind};
use super::tracks::NAME_COLUMN_WIDTH;

pub const LANE_HEIGHT: f32 = 64.0;
const RULER_HEIGHT: f32 = 18.0;
const DEFAULT_NEW_CHORD_BEATS: u32 = 4;
/// Horizontal pixels from a chord block's right edge that count as the
/// resize handle. Anything inside that strip starts a resize drag; the rest
/// of the body starts a move drag.
const RESIZE_HANDLE_PX: f32 = 8.0;

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
    drag: Option<ChordDrag>,
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
enum ChordDrag {
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
    fn draw_into(&self, frame: &mut Frame, bounds: Rectangle, drag: &Option<ChordDrag>) {
        // ---- Lane side panel ----
        let chord_count = self.definition.chords.len();
        let scale_word = match (chord_count, self.definition.scale.as_ref()) {
            (n, Some(scale)) => format!("{} \u{00b7} {} chords", scale.root, n),
            (n, None) => format!("{} chords", n),
        };
        lane_side::draw(
            frame,
            Rectangle {
                x: 0.0,
                y: 0.0,
                width: NAME_COLUMN_WIDTH,
                height: bounds.height,
            },
            LaneKind::Harmony,
            "Chords",
            Some(&scale_word),
            self.chords_selected,
        );

        // ---- Grid area (right of name column) ----
        let grid_x = NAME_COLUMN_WIDTH;
        let grid_w = (bounds.width - NAME_COLUMN_WIDTH).max(0.0);

        frame.fill_rectangle(
            Point::new(grid_x, 0.0),
            Size::new(grid_w, bounds.height),
            theme::BG_1,
        );

        let total_beats = self.total_beats();
        if total_beats == 0 || grid_w <= 0.0 {
            return;
        }
        let beat_width = grid_w / total_beats as f32;

        // Ruler ticks + bar numbers — walk bars for correct placement
        // with varying time signatures.
        let mut beat_pos: u32 = 0;
        for bar_offset in 0..self.definition.length_bars {
            let bar = self.start_bar + bar_offset;
            let num = self.tempo_map.numerator_at_bar(bar) as u32;

            // Bar line
            let x = grid_x + beat_pos as f32 * beat_width;
            frame.stroke(
                &Path::line(Point::new(x, 0.0), Point::new(x, RULER_HEIGHT)),
                Stroke::default().with_width(1.0).with_color(theme::TEXT_DIM),
            );
            // Bar number
            frame.fill_text(canvas::Text {
                content: format!("{}", bar_offset + 1),
                position: Point::new(x + 3.0, 2.0),
                color: theme::TEXT_DIM,
                size: 10.0.into(),
                ..canvas::Text::default()
            });

            // Beat ticks within this bar
            for beat in 1..num {
                let bx = grid_x + (beat_pos + beat) as f32 * beat_width;
                frame.stroke(
                    &Path::line(Point::new(bx, 0.0), Point::new(bx, RULER_HEIGHT * 0.5)),
                    Stroke::default()
                        .with_width(1.0)
                        .with_color(theme::SEPARATOR),
                );
            }

            beat_pos += num;
        }
        // Final bar line at section end
        let x = grid_x + beat_pos as f32 * beat_width;
        frame.stroke(
            &Path::line(Point::new(x, 0.0), Point::new(x, RULER_HEIGHT)),
            Stroke::default().with_width(1.0).with_color(theme::TEXT_DIM),
        );

        // Separator between ruler and chord area
        frame.fill_rectangle(
            Point::new(grid_x, RULER_HEIGHT),
            Size::new(grid_w, 1.0),
            theme::SEPARATOR,
        );

        // Chord blocks (with drag preview overrides). The redesign frames
        // each chord as a rounded card with a lavender wash + border;
        // selected/dragging cards get a stronger lavender border.
        let block_top = RULER_HEIGHT + 4.0;
        let block_h = bounds.height - block_top - 4.0;
        for chord in &self.definition.chords {
            let (start, dur) = apply_drag_preview(
                chord.id,
                chord.start_beat,
                chord.duration_beats,
                drag,
            );
            let x = grid_x + start as f32 * beat_width + 1.0;
            let w = (dur as f32 * beat_width - 2.0).max(2.0);
            let selected = Some(chord.id) == self.selected_chord_id;
            let dragging = matches!(drag, Some(ChordDrag::Move { chord_id, .. } | ChordDrag::Resize { chord_id, .. }) if *chord_id == chord.id);

            let fill = if selected || dragging {
                Color {
                    a: 0.22,
                    ..theme::ACCENT
                }
            } else {
                theme::BG_2
            };
            let border = if selected || dragging {
                theme::ACCENT
            } else {
                theme::LINE_2
            };
            let card = Path::rounded_rectangle(
                Point::new(x, block_top),
                Size::new(w, block_h),
                8.0.into(),
            );
            frame.fill(&card, fill);
            frame.stroke(
                &card,
                Stroke::default()
                    .with_width(if selected || dragging { 1.5 } else { 1.0 })
                    .with_color(border),
            );

            // Right-edge resize hint (tiny vertical bar)
            if w > RESIZE_HANDLE_PX * 2.0 {
                frame.fill_rectangle(
                    Point::new(x + w - 4.0, block_top + 6.0),
                    Size::new(2.0, block_h - 12.0),
                    Color {
                        a: 0.28,
                        ..theme::TEXT_2
                    },
                );
            }

            // Roman-numeral degree (small, mono, top-left). Computed
            // inline against the section's scale; "—" if no scale or the
            // chord root isn't on the scale.
            let degree = roman_numeral_for(&chord.chord, self.definition.scale.as_ref());
            frame.fill_text(canvas::Text {
                content: degree,
                position: Point::new(x + 8.0, block_top + 4.0),
                color: theme::TEXT_3,
                size: 9.0.into(),
                font: theme::MONO_FONT,
                ..canvas::Text::default()
            });

            // Chord symbol — italic serif, primary text color.
            frame.fill_text(canvas::Text {
                content: chord.chord.to_string(),
                position: Point::new(x + 8.0, block_top + 16.0),
                color: theme::TEXT_1,
                size: 18.0.into(),
                font: theme::SERIF_ITALIC_FONT,
                ..canvas::Text::default()
            });
        }

        // Bottom separator
        frame.fill_rectangle(
            Point::new(0.0, bounds.height - 1.0),
            Size::new(bounds.width, 1.0),
            theme::SEPARATOR,
        );
    }
}

impl<'a> ChordLaneCanvas<'a> {
    fn update_inner(
        &self,
        state: &mut ChordLaneState,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let grid_x = NAME_COLUMN_WIDTH;
        let grid_w = (bounds.width - NAME_COLUMN_WIDTH).max(0.0);
        let total_beats = self.total_beats();
        if total_beats == 0 || grid_w <= 0.0 {
            return None;
        }
        let beat_width = grid_w / total_beats as f32;

        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(pos) = cursor.position_in(bounds) else {
                    return None;
                };

                // Click on the name column: select the chords lane.
                if pos.x < NAME_COLUMN_WIDTH {
                    return Some(canvas::Action::publish(Message::Compose(ComposeMessage::SelectLane(
                            SelectedLane::Chords,
                        ))).and_capture());
                }

                if pos.y < RULER_HEIGHT {
                    return None;
                }
                let rel_x = pos.x - grid_x;
                let beat = (rel_x / beat_width) as u32;
                if beat >= total_beats {
                    return None;
                }

                // Hit-test existing chords: right edge => resize, body => move.
                for chord in &self.definition.chords {
                    let end = chord.start_beat + chord.duration_beats;
                    if beat >= chord.start_beat && beat < end {
                        let chord_right_px = grid_x + end as f32 * beat_width;
                        if chord_right_px - pos.x <= RESIZE_HANDLE_PX && chord.duration_beats >= 1 {
                            state.drag = Some(ChordDrag::Resize {
                                chord_id: chord.id,
                                pending_duration_beats: chord.duration_beats,
                            });
                        } else {
                            let grab_beat = beat.saturating_sub(chord.start_beat);
                            state.drag = Some(ChordDrag::Move {
                                chord_id: chord.id,
                                grab_beat,
                                pending_start_beat: chord.start_beat,
                            });
                        }
                        return Some(canvas::Action::publish(Message::Compose(ComposeMessage::SelectChord {
                                chord_id: chord.id,
                            })).and_capture());
                    }
                }

                // Empty slot: add a default C-major chord covering up to
                // DEFAULT_NEW_CHORD_BEATS beats without overrunning the next
                // chord or the section end.
                let mut duration = DEFAULT_NEW_CHORD_BEATS.min(total_beats - beat);
                for chord in &self.definition.chords {
                    if chord.start_beat > beat {
                        let gap = chord.start_beat - beat;
                        if gap < duration {
                            duration = gap;
                        }
                        break;
                    }
                }
                if duration == 0 {
                    return None;
                }
                Some(canvas::Action::publish(Message::Compose(ComposeMessage::AddChord {
                        definition_id: self.definition.id,
                        start_beat: beat,
                        duration_beats: duration,
                        root: PitchClass::C,
                        quality: ChordQuality::Maj,
                    })).and_capture())
            }

            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let Some(pos) = cursor.position_in(bounds) else {
                    return None;
                };
                let Some(drag) = state.drag.as_mut() else {
                    return None;
                };
                let rel_x = (pos.x - grid_x).max(0.0);
                let beat_f = (rel_x / beat_width).max(0.0);
                let beat = (beat_f as u32).min(total_beats.saturating_sub(1));
                match drag {
                    ChordDrag::Move {
                        chord_id,
                        grab_beat,
                        pending_start_beat,
                    } => {
                        let chord = match self.definition.chords.iter().find(|c| c.id == *chord_id)
                        {
                            Some(c) => c,
                            None => return None,
                        };
                        let new_start = beat.saturating_sub(*grab_beat);
                        let max_start = total_beats.saturating_sub(chord.duration_beats);
                        *pending_start_beat = new_start.min(max_start);
                        Some(canvas::Action::capture())
                    }
                    ChordDrag::Resize {
                        chord_id,
                        pending_duration_beats,
                    } => {
                        let chord = match self.definition.chords.iter().find(|c| c.id == *chord_id)
                        {
                            Some(c) => c,
                            None => return None,
                        };
                        let end_beat = ((rel_x / beat_width).ceil() as u32).min(total_beats);
                        let new_dur = end_beat.saturating_sub(chord.start_beat).max(1);
                        *pending_duration_beats = new_dur;
                        Some(canvas::Action::capture())
                    }
                }
            }

            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let drag = state.drag.take();
                match drag {
                    Some(ChordDrag::Move {
                        chord_id,
                        pending_start_beat,
                        ..
                    }) => {
                        let current = self
                            .definition
                            .chords
                            .iter()
                            .find(|c| c.id == chord_id)
                            .map(|c| c.start_beat);
                        if current == Some(pending_start_beat) {
                            return Some(canvas::Action::capture());
                        }
                        Some(canvas::Action::publish(Message::Compose(ComposeMessage::MoveChord {
                                definition_id: self.definition.id,
                                chord_id,
                                start_beat: pending_start_beat,
                            })).and_capture())
                    }
                    Some(ChordDrag::Resize {
                        chord_id,
                        pending_duration_beats,
                    }) => {
                        let current = self
                            .definition
                            .chords
                            .iter()
                            .find(|c| c.id == chord_id)
                            .map(|c| c.duration_beats);
                        if current == Some(pending_duration_beats) {
                            return Some(canvas::Action::capture());
                        }
                        Some(canvas::Action::publish(Message::Compose(ComposeMessage::ResizeChord {
                                definition_id: self.definition.id,
                                chord_id,
                                duration_beats: pending_duration_beats,
                            })).and_capture())
                    }
                    None => None,
                }
            }

            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                let Some(pos) = cursor.position_in(bounds) else {
                    return None;
                };
                if pos.x < NAME_COLUMN_WIDTH || pos.y < RULER_HEIGHT {
                    return None;
                }
                let rel_x = pos.x - grid_x;
                let beat = (rel_x / beat_width) as u32;
                for chord in &self.definition.chords {
                    let end = chord.start_beat + chord.duration_beats;
                    if beat >= chord.start_beat && beat < end {
                        return Some(canvas::Action::publish(Message::Compose(ComposeMessage::DeleteChord {
                                definition_id: self.definition.id,
                                chord_id: chord.id,
                            })).and_capture());
                    }
                }
                None
            }

            _ => None,
        }
    }
}

impl<'a> ChordLaneCanvas<'a> {
    /// Total beats in the section, summing per-bar numerators.
    fn total_beats(&self) -> u32 {
        (0..self.definition.length_bars)
            .map(|b| self.tempo_map.numerator_at_bar(self.start_bar + b) as u32)
            .sum()
    }
}

/// Roman-numeral degree label for a chord against its section's scale.
/// Lowercase numerals for minor / diminished qualities; "—" when the
/// scale is unknown or the chord root isn't a diatonic degree.
fn roman_numeral_for(
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
fn apply_drag_preview(
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
