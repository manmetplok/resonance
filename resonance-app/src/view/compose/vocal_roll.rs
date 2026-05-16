//! Vocal roll — the piano-roll-style editor that opens when a vocal lane
//! is double-clicked.
//!
//! Mirrors `crate::midi_editor::PianoRollCanvas` but tuned for vocals:
//!
//! * **Warm accent** on note bodies (matches the rest of the vocal UI).
//! * **Chord context strip** above the grid (read-only) — the singer
//!   reads the chord above the bar they're singing over.
//! * **Phoneme strip** under the chord strip — per-syllable ARPAbet
//!   phonemes from `g2p::phonemes_for_draft`, lined up with their note.
//! * **Lyric on each note body** — italic-serif syllable painted on the
//!   note rectangle, matching the design's "notes carry lyrics" pattern.
//! * **Slur arcs** between adjacent notes that flow without a rest —
//!   the visual cue for legato phoneme transitions in the SVS render.
//! * **Pitch curve overlay** — the rendered f0 path the SVS engine will
//!   sing: piecewise-constant note pitches, linear portamento between
//!   them, sinusoidal vibrato wobble on sustained notes. Lets the user
//!   audition portamento / vibrato changes visually.
//! * **Voice-type-bounded keyboard** — only renders the rows inside
//!   `params.range` so the editor doesn't waste vertical space on
//!   octaves the part will never touch.
//!
//! Edits go through the same `MidiEditorMessage::{Add,Remove,Move,Resize}`
//! the piano roll uses; the engine command path is shared.

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use resonance_audio::types::{TrackId, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::{g2p, VocalParams};

use crate::compose::{ChordState, SectionDefinitionState};
use crate::message::{Message, MidiEditorMessage};
use crate::state::MidiClipState;
use crate::theme;

/// Width of the piano keyboard column.
pub const VR_KEYBOARD_WIDTH: f32 = 56.0;
/// Height of the chord-context strip above the note grid.
pub const VR_CHORD_STRIP_HEIGHT: f32 = 28.0;
/// Height of the phoneme strip directly above the note grid.
pub const VR_PHONEME_STRIP_HEIGHT: f32 = 22.0;
/// Height of the velocity lane below the note grid.
pub const VR_VELOCITY_LANE_HEIGHT: f32 = 44.0;
/// Stacked header height — chord + phoneme strips.
const HEADER_TOTAL_HEIGHT: f32 = VR_CHORD_STRIP_HEIGHT + VR_PHONEME_STRIP_HEIGHT;
/// Minimum resize threshold in pixels for the right edge of a note.
const RESIZE_EDGE_PX: f32 = 6.0;
/// Default velocity for newly drawn notes.
const DEFAULT_VELOCITY: f32 = 0.8;
/// Threshold (in ticks) under which two adjacent notes are considered
/// "flowing without a rest" — drives the slur-arc rendering.
const SLUR_GAP_TICKS: u64 = TICKS_PER_QUARTER_NOTE / 32;

/// Returns true if the given MIDI note number corresponds to a black key.
fn is_black_key(note: u8) -> bool {
    matches!(note % 12, 1 | 3 | 6 | 8 | 10)
}

/// "C4", "F#3", ...
fn note_name(note: u8) -> String {
    let names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (note / 12) as i8 - 1;
    format!("{}{}", names[note as usize % 12], octave)
}

/// Snapshot of everything the canvas needs to render and respond to
/// input. Built fresh from the app state every frame; the persistent
/// drag/preview state lives in `VocalRollState`.
#[derive(Debug)]
pub struct VocalRollCanvas<'a> {
    pub clip: &'a MidiClipState,
    pub track_id: TrackId,
    pub params: &'a VocalParams,
    /// Section chords + total beats — used to draw the read-only chord
    /// context strip above the note grid.
    pub chords: &'a [ChordState],
    pub section_beats: u32,
    pub scroll_y: f32,
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub snap_ticks: u64,
    pub selected_note: Option<usize>,
    pub time_sig_num: u8,
    /// Voice name shown in the top-left meta corner.
    pub voice_label: &'a str,
    /// Per-note lyrics from `compose.vocal_clip_lyrics`. Index i aligns
    /// with the i-th `MidiNote`. Slurs are identified by the lyric
    /// being equal to [`resonance_music_theory::VocalNote::SLUR_MARKER`]
    /// (`"+"`).
    pub lyrics: &'a [String],
}

/// Build a `VocalRollCanvas` from the `Compose` view's already-available
/// state. Returns `None` when the editing clip's track isn't a vocal
/// track or its section/definition can't be located.
pub fn build_canvas<'a>(
    app: &'a crate::Resonance,
    clip: &'a MidiClipState,
) -> Option<VocalRollCanvas<'a>> {
    let editor_state = app.interaction.editing_midi_clip.as_ref()?;
    let definition = find_definition_for_clip(app, clip)?;
    let params = find_vocal_params(definition, clip.track_id)?;
    let voice_label = params.voice.as_str();
    // Side-table lookup — falls back to an empty slice when the clip
    // has never had lyrics installed (e.g. a vocal clip created
    // through the engine without going through the generator). Callers
    // already pad to `clip.notes.len()` when they install, so when
    // present the slice length always matches the note count.
    static EMPTY_LYRICS: Vec<String> = Vec::new();
    let lyrics = app
        .compose
        .vocal_clip_lyrics
        .get(&clip.id)
        .map(|v| v.as_slice())
        .unwrap_or(EMPTY_LYRICS.as_slice());
    Some(VocalRollCanvas {
        clip,
        track_id: editor_state.track_id,
        params,
        chords: &definition.chords,
        section_beats: definition.length_bars * app.transport.time_sig_num as u32,
        scroll_y: editor_state.scroll_y,
        zoom_x: editor_state.zoom_x,
        zoom_y: editor_state.zoom_y,
        snap_ticks: editor_state.snap_ticks,
        selected_note: editor_state.selected_note,
        time_sig_num: app.transport.time_sig_num,
        voice_label,
        lyrics,
    })
}

/// Walk the compose state to find which section definition owns the
/// derived MIDI clip the user just opened. Falls back to scanning every
/// definition's lane generators for one keyed to the clip's track id.
fn find_definition_for_clip<'a>(
    app: &'a crate::Resonance,
    clip: &MidiClipState,
) -> Option<&'a SectionDefinitionState> {
    for ((def_id, _placement_id, track_id), cid) in app.compose.derived_clips.iter() {
        if *cid == clip.id && *track_id == clip.track_id {
            return app.compose.definitions.iter().find(|d| d.id == *def_id);
        }
    }
    // Fall back: any definition that has a vocal generator for this
    // track — this covers projects where the derived-clip map hasn't
    // been rebuilt yet (e.g. immediately after load).
    app.compose
        .definitions
        .iter()
        .find(|d| find_vocal_params(d, clip.track_id).is_some())
}

fn find_vocal_params(
    def: &SectionDefinitionState,
    track_id: TrackId,
) -> Option<&VocalParams> {
    use crate::compose::LaneGeneratorKind;
    def.lane_generators.get(&track_id).and_then(|cfg| match &cfg.kind {
        LaneGeneratorKind::Vocal(p) => Some(p),
        _ => None,
    })
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum DragMode {
    MoveNote {
        note_index: usize,
        start_tick_offset: i64,
        original_note: u8,
        original_start_tick: u64,
    },
    ResizeNote {
        note_index: usize,
        anchor_tick: u64,
    },
}

/// Local canvas state — only drag + preview live here; everything else
/// is read off `Resonance` every paint.
#[derive(Debug, Default)]
pub struct VocalRollState {
    drag: Option<DragMode>,
    previewing_note: Option<u8>,
    cache: canvas::Cache,
    cache_fingerprint: std::cell::Cell<VocalRollFingerprint>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VocalRollFingerprint {
    clip_id: u64,
    notes_len: usize,
    notes_hash: u64,
    scroll_y_bits: u32,
    zoom_x_bits: u32,
    zoom_y_bits: u32,
    snap_ticks: u64,
    selected_note: Option<usize>,
    time_sig_num: u8,
    drag_active: bool,
    preview_note: Option<u8>,
    range_lo: u8,
    range_hi: u8,
    chords_hash: u64,
    draft_hash: u64,
    lyrics_hash: u64,
}

impl canvas::Program<Message> for VocalRollCanvas<'_> {
    type State = VocalRollState;

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        let grid_x = VR_KEYBOARD_WIDTH;
        let grid_top = HEADER_TOTAL_HEIGHT;
        let grid_h = bounds.height - HEADER_TOTAL_HEIGHT - VR_VELOCITY_LANE_HEIGHT;
        let grid_bottom = grid_top + grid_h;

        match event {
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if cursor.position_in(bounds).is_none() {
                    return (canvas::event::Status::Ignored, None);
                }
                match delta {
                    mouse::ScrollDelta::Lines { x, y } => {
                        if x.abs() > f32::EPSILON {
                            return (canvas::event::Status::Ignored, None);
                        }
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::MidiEditor(MidiEditorMessage::ScrollY(-y * 30.0))),
                        );
                    }
                    mouse::ScrollDelta::Pixels { x, y } => {
                        if x.abs() > f32::EPSILON {
                            return (canvas::event::Status::Ignored, None);
                        }
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::MidiEditor(MidiEditorMessage::ScrollY(-y))),
                        );
                    }
                }
            }

            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    // Piano keyboard preview — only in the grid band.
                    if pos.x < grid_x && pos.y >= grid_top && pos.y < grid_bottom {
                        if let Some(note) = self.y_to_note(pos.y - grid_top, grid_h) {
                            state.previewing_note = Some(note);
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::MidiEditor(MidiEditorMessage::PreviewNote(
                                    self.track_id,
                                    note,
                                ))),
                            );
                        }
                    }

                    if pos.y < grid_top || pos.y >= grid_bottom {
                        // Velocity lane / chord strip / lyric strip are
                        // read-only for now — clicks just select the
                        // editor (no message).
                        return (canvas::event::Status::Ignored, None);
                    }

                    if pos.x >= grid_x {
                        let rel_x = pos.x - grid_x;
                        let rel_y = pos.y - grid_top;
                        let click_tick = self.x_to_tick(rel_x);
                        let Some(click_note) = self.y_to_note(rel_y, grid_h) else {
                            return (canvas::event::Status::Ignored, None);
                        };

                        for (i, n) in self.clip.notes.iter().enumerate() {
                            let nx = self.tick_to_x(n.start_tick);
                            let nw = self.duration_to_width(n.duration_ticks);
                            let Some(ny) = self.note_to_y(n.note, grid_h) else {
                                continue;
                            };
                            let nh = self.zoom_y;
                            if rel_x >= nx && rel_x <= nx + nw && rel_y >= ny && rel_y <= ny + nh
                            {
                                if (nx + nw) - rel_x < RESIZE_EDGE_PX {
                                    state.drag = Some(DragMode::ResizeNote {
                                        note_index: i,
                                        anchor_tick: n.start_tick,
                                    });
                                    return (
                                        canvas::event::Status::Captured,
                                        Some(Message::MidiEditor(MidiEditorMessage::SelectNote {
                                            note_index: Some(i),
                                        })),
                                    );
                                }
                                let tick_offset = n.start_tick as i64 - click_tick as i64;
                                state.drag = Some(DragMode::MoveNote {
                                    note_index: i,
                                    start_tick_offset: tick_offset,
                                    original_note: n.note,
                                    original_start_tick: n.start_tick,
                                });
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::MidiEditor(MidiEditorMessage::SelectNote {
                                        note_index: Some(i),
                                    })),
                                );
                            }
                        }

                        let snapped = self.snap(click_tick);
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::MidiEditor(MidiEditorMessage::AddNote {
                                clip_id: self.clip.id,
                                note: click_note,
                                start_tick: snapped,
                                duration_ticks: self.snap_ticks.max(TICKS_PER_QUARTER_NOTE / 4),
                                velocity: DEFAULT_VELOCITY,
                            })),
                        );
                    }
                }
            }

            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    if pos.x >= grid_x && pos.y >= grid_top && pos.y < grid_bottom {
                        let rel_x = pos.x - grid_x;
                        let rel_y = pos.y - grid_top;
                        for (i, n) in self.clip.notes.iter().enumerate() {
                            let nx = self.tick_to_x(n.start_tick);
                            let nw = self.duration_to_width(n.duration_ticks);
                            let Some(ny) = self.note_to_y(n.note, grid_h) else {
                                continue;
                            };
                            let nh = self.zoom_y;
                            if rel_x >= nx && rel_x <= nx + nw && rel_y >= ny && rel_y <= ny + nh
                            {
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::MidiEditor(MidiEditorMessage::RemoveNote {
                                        clip_id: self.clip.id,
                                        note_index: i,
                                    })),
                                );
                            }
                        }
                    }
                }
            }

            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    let rel_x = pos.x - grid_x;
                    let rel_y = pos.y - grid_top;
                    match &state.drag {
                        Some(DragMode::MoveNote {
                            note_index,
                            start_tick_offset,
                            ..
                        }) if pos.x >= grid_x && pos.y >= grid_top && pos.y < grid_bottom => {
                            let tick = self.x_to_tick(rel_x);
                            let raw_tick = (tick as i64 + start_tick_offset).max(0) as u64;
                            let snapped_tick = self.snap(raw_tick);
                            let Some(note) = self.y_to_note(rel_y, grid_h) else {
                                return (canvas::event::Status::Ignored, None);
                            };
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::MidiEditor(MidiEditorMessage::MoveNote {
                                    clip_id: self.clip.id,
                                    note_index: *note_index,
                                    new_start_tick: snapped_tick,
                                    new_note: note,
                                })),
                            );
                        }
                        Some(DragMode::ResizeNote {
                            note_index,
                            anchor_tick,
                        }) if pos.x >= grid_x => {
                            let tick = self.x_to_tick(rel_x);
                            let snapped = self.snap(tick);
                            let new_dur =
                                snapped.saturating_sub(*anchor_tick).max(self.snap_ticks);
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::MidiEditor(MidiEditorMessage::ResizeNote {
                                    clip_id: self.clip.id,
                                    note_index: *note_index,
                                    new_duration_ticks: new_dur,
                                })),
                            );
                        }
                        Some(_) | None => {}
                    }
                }
            }

            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag = None;
                if let Some(note) = state.previewing_note.take() {
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::MidiEditor(MidiEditorMessage::StopPreview(
                            self.track_id,
                            note,
                        ))),
                    );
                }
            }

            canvas::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Delete),
                ..
            })
            | canvas::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Backspace),
                ..
            }) => {
                if let Some(idx) = self.selected_note {
                    if idx < self.clip.notes.len() {
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::MidiEditor(MidiEditorMessage::RemoveNote {
                                clip_id: self.clip.id,
                                note_index: idx,
                            })),
                        );
                    }
                }
            }

            // OpenUtau-style slur toggle. Pressing `s` (or `+`) on the
            // selected note flips its lyric between the slur marker
            // and the auto-syllabified surface form. Mirrors the
            // shortcut users coming from OpenUtau / Vocaloid editors
            // expect.
            canvas::Event::Keyboard(iced::keyboard::Event::KeyPressed { ref text, .. }) => {
                if let Some(idx) = self.selected_note {
                    if idx < self.clip.notes.len() {
                        if let Some(t) = text.as_deref() {
                            let key = t.trim();
                            if key.eq_ignore_ascii_case("s") || key == "+" {
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::MidiEditor(MidiEditorMessage::ToggleSlur {
                                        clip_id: self.clip.id,
                                        note_index: idx,
                                    })),
                                );
                            }
                        }
                    }
                }
            }

            _ => {}
        }
        (canvas::event::Status::Ignored, None)
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let fp = self.fingerprint(state);
        if state.cache_fingerprint.get() != fp {
            state.cache.clear();
            state.cache_fingerprint.set(fp);
        }
        let geo = state.cache.draw(renderer, bounds.size(), |frame| {
            self.draw_into(frame, bounds);
        });
        vec![geo]
    }
}

impl VocalRollCanvas<'_> {
    fn fingerprint(&self, state: &VocalRollState) -> VocalRollFingerprint {
        use std::hash::{Hash, Hasher};
        let mut nh = std::collections::hash_map::DefaultHasher::new();
        for n in &self.clip.notes {
            n.note.hash(&mut nh);
            n.start_tick.hash(&mut nh);
            n.duration_ticks.hash(&mut nh);
            n.velocity.to_bits().hash(&mut nh);
        }
        let mut ch = std::collections::hash_map::DefaultHasher::new();
        for c in self.chords {
            c.start_beat.hash(&mut ch);
            c.duration_beats.hash(&mut ch);
            c.chord.root.to_semitone().hash(&mut ch);
        }
        let mut dh = std::collections::hash_map::DefaultHasher::new();
        for l in &self.params.draft {
            l.text.hash(&mut dh);
            l.syllables.hash(&mut dh);
        }
        let mut lh = std::collections::hash_map::DefaultHasher::new();
        for l in self.lyrics {
            l.hash(&mut lh);
        }
        let (lo, hi) = self.params.range;
        VocalRollFingerprint {
            clip_id: self.clip.id,
            notes_len: self.clip.notes.len(),
            notes_hash: nh.finish(),
            scroll_y_bits: self.scroll_y.to_bits(),
            zoom_x_bits: self.zoom_x.to_bits(),
            zoom_y_bits: self.zoom_y.to_bits(),
            snap_ticks: self.snap_ticks,
            selected_note: self.selected_note,
            time_sig_num: self.time_sig_num,
            drag_active: state.drag.is_some(),
            preview_note: state.previewing_note,
            range_lo: lo,
            range_hi: hi,
            chords_hash: ch.finish(),
            draft_hash: dh.finish(),
            lyrics_hash: lh.finish(),
        }
    }

    fn draw_into(&self, frame: &mut Frame, bounds: Rectangle) {
        let grid_x = VR_KEYBOARD_WIDTH;
        let grid_w = bounds.width - VR_KEYBOARD_WIDTH;
        let grid_top = HEADER_TOTAL_HEIGHT;
        let grid_h = bounds.height - HEADER_TOTAL_HEIGHT - VR_VELOCITY_LANE_HEIGHT;

        // Resolve syllables + phonemes once — both are indexed by note
        // position. Syllables come from the per-clip side-table (which
        // carries hand-edits and slur markers); the side-table falls
        // back to the auto-syllabified draft so legacy clips without
        // an installed table still get sensible labels.
        let syllables = effective_syllables(self);
        let phonemes = g2p::phonemes_for_draft(&self.params.draft);

        // Background
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::BG);

        // Top-left corner — voice label
        frame.fill_rectangle(
            Point::new(0.0, 0.0),
            Size::new(grid_x, HEADER_TOTAL_HEIGHT),
            theme::BG_2,
        );
        frame.fill_text(canvas::Text {
            content: self.voice_label.to_uppercase(),
            position: Point::new(8.0, 6.0),
            color: theme::WARM,
            size: 9.0.into(),
            font: theme::UI_FONT_SEMIBOLD,
            ..canvas::Text::default()
        });
        frame.fill_text(canvas::Text {
            content: format!(
                "{}-{}",
                note_name(self.params.range.0),
                note_name(self.params.range.1)
            ),
            position: Point::new(8.0, 20.0),
            color: theme::TEXT_3,
            size: 9.0.into(),
            font: theme::MONO_FONT,
            ..canvas::Text::default()
        });

        // Chord strip + phoneme strip across the grid width.
        self.draw_chord_strip(frame, grid_x, grid_w);
        self.draw_phoneme_strip(frame, grid_x, grid_w, &phonemes);

        // Note row backgrounds
        self.draw_note_rows(frame, grid_x, grid_w, grid_top, grid_h);

        // Bar / beat lines
        self.draw_grid_lines(frame, grid_x, grid_w, grid_top, grid_h);

        // Notes — drawn before slurs/pitch curve so those overlay on top.
        self.draw_notes(frame, grid_x, grid_top, grid_h, &syllables);

        // Slur arcs between adjacent flowing notes.
        self.draw_slurs(frame, grid_x, grid_top, grid_h);

        // Rendered f0 contour overlay (portamento + vibrato).
        self.draw_pitch_curve(frame, grid_x, grid_w, grid_top, grid_h);

        // Piano keyboard
        self.draw_keyboard(frame, grid_top, grid_h);

        // Velocity lane
        self.draw_velocity_lane(frame, grid_x, grid_w, bounds.height);

        // Separator lines
        frame.fill_rectangle(
            Point::new(grid_x, 0.0),
            Size::new(1.0, bounds.height - VR_VELOCITY_LANE_HEIGHT),
            theme::SEPARATOR,
        );
        frame.fill_rectangle(
            Point::new(0.0, grid_top + grid_h),
            Size::new(bounds.width, 1.0),
            theme::SEPARATOR,
        );
        frame.fill_rectangle(
            Point::new(0.0, VR_CHORD_STRIP_HEIGHT),
            Size::new(bounds.width, 1.0),
            theme::LINE_2,
        );
        frame.fill_rectangle(
            Point::new(0.0, HEADER_TOTAL_HEIGHT),
            Size::new(bounds.width, 1.0),
            theme::SEPARATOR,
        );
    }
}

impl VocalRollCanvas<'_> {
    fn tick_to_x(&self, tick: u64) -> f32 {
        tick as f32 * self.zoom_x
    }

    fn x_to_tick(&self, x: f32) -> u64 {
        if x <= 0.0 {
            0
        } else {
            (x / self.zoom_x) as u64
        }
    }

    fn duration_to_width(&self, ticks: u64) -> f32 {
        ticks as f32 * self.zoom_x
    }

    /// Convert MIDI note number to pixel y inside the *grid band*
    /// (relative to grid_top). Returns the top of the row.
    /// Notes outside `params.range` map outside the band — callers
    /// clip them.
    fn note_to_y(&self, note: u8, _grid_h: f32) -> Option<f32> {
        let (lo, hi) = self.params.range;
        if note < lo || note > hi {
            return None;
        }
        let row = (hi - note) as f32; // top row is hi
        Some(row * self.zoom_y - self.scroll_y)
    }

    /// Inverse — pixel y inside the grid band → MIDI note number, or
    /// `None` if outside the visible row range. Allows scroll past the
    /// top/bottom edges.
    fn y_to_note(&self, y: f32, _grid_h: f32) -> Option<u8> {
        let (lo, hi) = self.params.range;
        let row = ((y + self.scroll_y) / self.zoom_y).floor() as i32;
        if row < 0 {
            return None;
        }
        let note = hi as i32 - row;
        if note < lo as i32 || note > hi as i32 {
            return None;
        }
        Some(note as u8)
    }

    fn snap(&self, tick: u64) -> u64 {
        if self.snap_ticks == 0 {
            return tick;
        }
        let half = self.snap_ticks / 2;
        ((tick + half) / self.snap_ticks) * self.snap_ticks
    }

    fn draw_note_rows(&self, frame: &mut Frame, grid_x: f32, grid_w: f32, grid_top: f32, grid_h: f32) {
        // Backdrop
        frame.fill_rectangle(
            Point::new(grid_x, grid_top),
            Size::new(grid_w, grid_h),
            theme::BG_2,
        );
        let (lo, hi) = self.params.range;
        for note in lo..=hi {
            let Some(y_local) = self.note_to_y(note, grid_h) else {
                continue;
            };
            let y = grid_top + y_local;
            let h = self.zoom_y;
            if y + h < grid_top || y > grid_top + grid_h {
                continue;
            }
            if is_black_key(note) {
                frame.fill_rectangle(Point::new(grid_x, y), Size::new(grid_w, h), theme::BG_1);
            }
            if note % 12 == 0 {
                frame.fill_rectangle(
                    Point::new(grid_x, y + h - 1.0),
                    Size::new(grid_w, 1.0),
                    theme::LINE_2,
                );
            }
        }
    }

    fn draw_grid_lines(
        &self,
        frame: &mut Frame,
        grid_x: f32,
        grid_w: f32,
        grid_top: f32,
        grid_h: f32,
    ) {
        let ticks_per_beat = TICKS_PER_QUARTER_NOTE;
        let ticks_per_bar = TICKS_PER_QUARTER_NOTE * self.time_sig_num as u64;
        let pixels_per_beat = ticks_per_beat as f32 * self.zoom_x;
        if pixels_per_beat < 4.0 {
            return;
        }
        let max_tick = ((grid_w / self.zoom_x) as u64) + ticks_per_beat;
        let mut tick = 0u64;
        while tick <= max_tick {
            let x = grid_x + self.tick_to_x(tick);
            if x > grid_x + grid_w {
                break;
            }
            let is_bar = tick.is_multiple_of(ticks_per_bar);
            let color = if is_bar { theme::BAR_LINE } else { theme::BEAT_LINE };
            frame.fill_rectangle(
                Point::new(x, grid_top),
                Size::new(1.0, grid_h),
                color,
            );
            tick = tick.saturating_add(ticks_per_beat);
        }

        // 16th sub-divisions when zoomed in.
        let snap_px = self.snap_ticks as f32 * self.zoom_x;
        if snap_px >= 8.0 && self.snap_ticks < ticks_per_beat {
            let mut tick = 0u64;
            while tick <= max_tick {
                if !tick.is_multiple_of(ticks_per_beat) {
                    let x = grid_x + self.tick_to_x(tick);
                    if x > grid_x + grid_w {
                        break;
                    }
                    frame.fill_rectangle(
                        Point::new(x, grid_top),
                        Size::new(1.0, grid_h),
                        Color { a: 0.5, ..theme::LINE_2 },
                    );
                }
                tick = tick.saturating_add(self.snap_ticks);
            }
        }
    }

    fn draw_notes(
        &self,
        frame: &mut Frame,
        grid_x: f32,
        grid_top: f32,
        grid_h: f32,
        syllables: &[String],
    ) {
        for (i, n) in self.clip.notes.iter().enumerate() {
            let Some(y_local) = self.note_to_y(n.note, grid_h) else {
                continue;
            };
            let x = grid_x + self.tick_to_x(n.start_tick);
            let w = self.duration_to_width(n.duration_ticks);
            let y = grid_top + y_local;
            let h = self.zoom_y;
            if y + h < grid_top || y > grid_top + grid_h {
                continue;
            }
            let v = n.velocity.clamp(0.0, 1.0);
            let is_slur = is_slur_note(self.lyrics, i);
            // Slur notes paint thinner and more transparent — visually
            // says "this isn't a new attack, just a pitch change inside
            // the previous syllable". Matches the engraving convention
            // where slurred notes share a beam / common phrase.
            let body_color = if is_slur {
                Color {
                    a: 0.30 + 0.25 * v,
                    ..theme::WARM
                }
            } else {
                Color {
                    a: 0.55 + 0.40 * v,
                    ..theme::WARM
                }
            };
            let body = if w >= 4.0 && h >= 4.0 {
                Path::rounded_rectangle(Point::new(x, y), Size::new(w, h), 2.0.into())
            } else {
                Path::rectangle(Point::new(x, y), Size::new(w, h))
            };
            frame.fill(&body, body_color);
            let selected = self.selected_note == Some(i);
            let stroke_color = if selected {
                theme::WARM
            } else if is_slur {
                Color { a: 0.60, ..theme::WARM }
            } else {
                theme::WARM_LINE
            };
            let stroke_w = if selected { 1.5 } else { 1.0 };
            frame.stroke(
                &body,
                Stroke::default().with_color(stroke_color).with_width(stroke_w),
            );
            // Selected: outer glow ring.
            if selected {
                frame.stroke(
                    &Path::rounded_rectangle(
                        Point::new(x - 1.5, y - 1.5),
                        Size::new(w + 3.0, h + 3.0),
                        3.0.into(),
                    ),
                    Stroke::default()
                        .with_color(Color { a: 0.45, ..theme::WARM })
                        .with_width(1.0),
                );
            }

            // Slur notes get a thin dashed top edge — visual "tie"
            // affordance that reads even when the slur arc above is
            // outside the viewport.
            if is_slur && w >= 6.0 {
                let dash_y = y + 1.0;
                let dash_count = ((w / 4.0) as usize).max(1);
                let dash_step = w / dash_count as f32;
                for d in 0..dash_count {
                    let dx = x + d as f32 * dash_step + dash_step * 0.2;
                    let dw = (dash_step * 0.5).max(1.0);
                    frame.fill_rectangle(
                        Point::new(dx, dash_y),
                        Size::new(dw, 1.0),
                        theme::WARM,
                    );
                }
            }

            // Syllable / slur marker on the note body — only when the
            // note is wide enough for the text to fit. Italic serif
            // matches the rest of the vocal UI; ink colour is dark
            // against the warm body for legibility. Slur notes show
            // the `+` marker centred (smaller) so the visual reads as
            // a continuation rather than a syllable.
            let label = syllables.get(i).cloned().unwrap_or_default();
            if !label.is_empty() && w >= 12.0 && h >= 10.0 {
                let (size, dx) = if is_slur {
                    ((h * 0.7).min(11.0), w * 0.5 - 3.0)
                } else {
                    ((h * 0.85).min(13.0), 4.0)
                };
                frame.fill_text(canvas::Text {
                    content: label,
                    position: Point::new(x + dx, y - 1.0),
                    color: if is_slur {
                        Color::from_rgba(0.08, 0.06, 0.04, 0.7)
                    } else {
                        Color::from_rgba(0.08, 0.06, 0.04, 0.92)
                    },
                    size: size.into(),
                    font: theme::SERIF_ITALIC_FONT,
                    ..canvas::Text::default()
                });
            }
        }
    }

    /// Slur arcs between adjacent notes. The arc is drawn between every
    /// pair `(a, b)` where `b` is explicitly marked as a slur (`lyric ==
    /// "+"`) — that's the OpenUtau convention the SVS pipeline reads.
    /// As a fallback for legacy clips without a lyric side-table, also
    /// draws an arc when two notes flow without a rest (gap below
    /// `SLUR_GAP_TICKS`). The arc rises above the higher of the two
    /// notes — standard engraving for legato.
    fn draw_slurs(&self, frame: &mut Frame, grid_x: f32, grid_top: f32, grid_h: f32) {
        if self.clip.notes.len() < 2 {
            return;
        }
        let has_lyrics = !self.lyrics.is_empty();
        for (i, win) in self.clip.notes.windows(2).enumerate() {
            let (a, b) = (&win[0], &win[1]);
            let b_index = i + 1;
            let connected = if has_lyrics {
                // Side-table available — only draw when the *next*
                // note is explicitly tagged as a slur. Stops the
                // editor from showing speculative arcs the SVS will
                // ignore anyway.
                is_slur_note(self.lyrics, b_index)
            } else {
                // Legacy clip — fall back to the proximity heuristic
                // so the visual still appears for in-progress files.
                let end_a = a.start_tick + a.duration_ticks;
                b.start_tick <= end_a + SLUR_GAP_TICKS
            };
            if !connected {
                continue;
            }
            let Some(ay) = self.note_to_y(a.note, grid_h) else { continue };
            let Some(by) = self.note_to_y(b.note, grid_h) else { continue };
            let ax = grid_x + self.tick_to_x(a.start_tick + a.duration_ticks);
            let bx = grid_x + self.tick_to_x(b.start_tick);
            let top_y_local = ay.min(by) - 4.0;
            let mid_x = (ax + bx) * 0.5;
            let p_a = Point::new(ax, grid_top + ay);
            let p_b = Point::new(bx, grid_top + by);
            let arc = Path::new(|p| {
                p.move_to(p_a);
                p.quadratic_curve_to(Point::new(mid_x, grid_top + top_y_local - 6.0), p_b);
            });
            frame.stroke(
                &arc,
                Stroke::default()
                    .with_color(Color { a: 0.85, ..theme::WARM })
                    .with_width(1.4),
            );
        }
    }

    /// Synthesise the f0 path the SVS engine will sing and draw it as a
    /// thin overlay on the note grid. Walks the notes back-to-back,
    /// interpolating pitch across `portamento_ms` between adjacent notes
    /// and applying a sinusoidal vibrato wobble in the sustain tail of
    /// long notes. This isn't the engine's exact f0 (which depends on
    /// model behaviour) but it's the same formula the pipeline uses to
    /// generate its starting f0_seq, so what the user sees is what
    /// they'll hear modulo the model's own micro-variations.
    fn draw_pitch_curve(
        &self,
        frame: &mut Frame,
        grid_x: f32,
        grid_w: f32,
        grid_top: f32,
        _grid_h: f32,
    ) {
        if self.clip.notes.is_empty() {
            return;
        }
        // Sample density — one sample per 2 px. The curve gets resampled
        // anyway by Iced; this just controls how many control points
        // the polyline has before the renderer takes over.
        let px_per_sample: f32 = 2.0;
        let total_w = grid_w.max(1.0);
        let samples = (total_w / px_per_sample) as usize;
        if samples < 4 {
            return;
        }
        // Pre-compute the portamento radius in ticks. Convert `ms` to
        // beats at the section's BPM. The chord strip already lays out
        // beats by `tick_to_x`, so converting to ticks keeps us in the
        // same axis space.
        let bpm = 90.0_f32.max(1.0); // best-effort default; the curve
        // is preview-quality so a slight BPM drift between sections is
        // fine — the absolute portamento duration only matters for
        // engine audio.
        let portamento_ticks =
            ((self.params.portamento_ms / 1000.0) * (bpm / 60.0) * TICKS_PER_QUARTER_NOTE as f32)
                .max(0.0) as u64;
        let vibrato_depth_st = (self.params.vibrato * 0.45).clamp(0.0, 0.45);
        let vibrato_rate_hz = self.params.vibrato_rate.clamp(2.0, 9.0);

        // The view-space y for a (possibly fractional) MIDI pitch.
        let pitch_y_view = |midi: f32| -> Option<f32> {
            let (lo, hi) = self.params.range;
            if midi < lo as f32 - 1.0 || midi > hi as f32 + 1.0 {
                return None;
            }
            let row = hi as f32 - midi;
            Some(grid_top + (row * self.zoom_y - self.scroll_y) + self.zoom_y * 0.5)
        };

        // Find the rendered pitch at tick `t` — piecewise constant note
        // pitch, with linear portamento ramps that *finish* at each
        // note start tick (matches the engine's portamento_frames
        // back-fill).
        let pitch_at = |t: u64| -> Option<f32> {
            // Find the note covering t (or the previous note for gaps).
            let mut prev: Option<&resonance_audio::types::MidiNote> = None;
            let mut cur: Option<&resonance_audio::types::MidiNote> = None;
            for n in &self.clip.notes {
                if n.start_tick <= t {
                    prev = cur;
                    cur = Some(n);
                } else {
                    break;
                }
            }
            let cur = cur?;
            // Portamento ramp: the last `portamento_ticks` before the
            // current note's start tick are blended from prev → cur.
            if let Some(p) = prev {
                let ramp_start = cur.start_tick.saturating_sub(portamento_ticks);
                if t >= ramp_start && t < cur.start_tick && portamento_ticks > 0 {
                    let span = (cur.start_tick - ramp_start) as f32;
                    let local = (t - ramp_start) as f32;
                    let alpha = (local / span).clamp(0.0, 1.0);
                    return Some(p.note as f32 * (1.0 - alpha) + cur.note as f32 * alpha);
                }
            }
            Some(cur.note as f32)
        };

        // Walk samples and build a path. Sustains add vibrato in the
        // back half of the note.
        let beats_per_sec = bpm / 60.0;
        let ticks_per_sec = beats_per_sec * TICKS_PER_QUARTER_NOTE as f32;
        let path = Path::new(|p| {
            let mut started = false;
            for i in 0..=samples {
                let x_local = i as f32 * px_per_sample;
                let tick = self.x_to_tick(x_local);
                let Some(mut midi) = pitch_at(tick) else { continue };
                // Vibrato: only when we're inside a sustained note,
                // 60 ms into its duration (matches the engine's onset
                // gate so onsets read clean).
                if let Some(n) = self
                    .clip
                    .notes
                    .iter()
                    .find(|n| n.start_tick <= tick && tick < n.start_tick + n.duration_ticks)
                {
                    let onset_ticks =
                        (0.06 * ticks_per_sec) as u64;
                    let elapsed = tick.saturating_sub(n.start_tick);
                    if elapsed > onset_ticks && n.duration_ticks > onset_ticks + 1 {
                        let t_sec = (elapsed - onset_ticks) as f32 / ticks_per_sec;
                        let wobble = (t_sec * vibrato_rate_hz * std::f32::consts::TAU).sin();
                        midi += vibrato_depth_st * wobble;
                    }
                }
                let Some(y) = pitch_y_view(midi) else { continue };
                let x = grid_x + x_local;
                if x > grid_x + grid_w {
                    break;
                }
                if !started {
                    p.move_to(Point::new(x, y));
                    started = true;
                } else {
                    p.line_to(Point::new(x, y));
                }
            }
        });
        frame.stroke(
            &path,
            Stroke::default()
                .with_color(Color::from_rgba(1.0, 0.95, 0.78, 0.85))
                .with_width(1.2),
        );
    }

    fn draw_keyboard(&self, frame: &mut Frame, grid_top: f32, grid_h: f32) {
        frame.fill_rectangle(
            Point::new(0.0, grid_top),
            Size::new(VR_KEYBOARD_WIDTH, grid_h),
            theme::BG_2,
        );
        let (lo, hi) = self.params.range;
        for note in lo..=hi {
            let Some(y_local) = self.note_to_y(note, grid_h) else {
                continue;
            };
            let y = grid_top + y_local;
            let h = self.zoom_y;
            if y + h < grid_top || y > grid_top + grid_h {
                continue;
            }
            let black = is_black_key(note);
            let key_color = if black { theme::BG_0 } else { theme::BG_3 };
            let key_w = if black {
                VR_KEYBOARD_WIDTH * 0.6
            } else {
                VR_KEYBOARD_WIDTH - 1.0
            };
            frame.fill_rectangle(
                Point::new(0.0, y),
                Size::new(key_w, h - 1.0),
                key_color,
            );
            // Label only on C rows when there's headroom.
            if note % 12 == 0 && h >= 8.0 {
                frame.fill_text(canvas::Text {
                    content: note_name(note),
                    position: Point::new(3.0, y + 1.0),
                    color: theme::TEXT_3,
                    size: (h * 0.7).min(10.0).into(),
                    font: theme::MONO_FONT,
                    ..canvas::Text::default()
                });
            }
        }
        // Right edge separator
        frame.fill_rectangle(
            Point::new(VR_KEYBOARD_WIDTH - 1.0, grid_top),
            Size::new(1.0, grid_h),
            theme::LINE_2,
        );
    }

    fn draw_velocity_lane(&self, frame: &mut Frame, grid_x: f32, grid_w: f32, total_h: f32) {
        let lane_y = total_h - VR_VELOCITY_LANE_HEIGHT;
        // Left label column
        frame.fill_rectangle(
            Point::new(0.0, lane_y),
            Size::new(grid_x + grid_w, VR_VELOCITY_LANE_HEIGHT),
            theme::PANEL_DARK,
        );
        frame.fill_text(canvas::Text {
            content: "VEL".to_string(),
            position: Point::new(8.0, lane_y + 4.0),
            color: theme::TEXT_3,
            size: 9.0.into(),
            font: theme::UI_FONT_SEMIBOLD,
            ..canvas::Text::default()
        });

        // Baseline
        frame.fill_rectangle(
            Point::new(grid_x, lane_y + VR_VELOCITY_LANE_HEIGHT - 2.0),
            Size::new(grid_w, 1.0),
            theme::LINE_2,
        );

        for (i, n) in self.clip.notes.iter().enumerate() {
            let x = grid_x + self.tick_to_x(n.start_tick);
            if x > grid_x + grid_w {
                break;
            }
            let w = self.duration_to_width(n.duration_ticks).clamp(2.0, 6.0);
            let bar_h = n.velocity.clamp(0.0, 1.0) * (VR_VELOCITY_LANE_HEIGHT - 8.0);
            let bar_y = lane_y + VR_VELOCITY_LANE_HEIGHT - bar_h - 4.0;
            let color = if self.selected_note == Some(i) {
                theme::WARM
            } else {
                Color { a: 0.65, ..theme::WARM }
            };
            frame.fill_rectangle(Point::new(x, bar_y), Size::new(w, bar_h), color);
        }
    }

    /// Read-only chord context strip aligned to the section's beat
    /// timeline so chord boundaries land on bar lines.
    fn draw_chord_strip(&self, frame: &mut Frame, grid_x: f32, grid_w: f32) {
        // Background — warm tint.
        frame.fill_rectangle(
            Point::new(grid_x, 0.0),
            Size::new(grid_w, VR_CHORD_STRIP_HEIGHT),
            Color { a: 0.06, ..theme::WARM },
        );
        let ticks_per_beat = TICKS_PER_QUARTER_NOTE;
        let section_ticks = (self.section_beats as u64) * ticks_per_beat;
        if section_ticks == 0 {
            return;
        }
        for c in self.chords {
            let start_tick = c.start_beat as u64 * ticks_per_beat;
            let dur_tick = c.duration_beats as u64 * ticks_per_beat;
            let x0 = grid_x + self.tick_to_x(start_tick);
            let w = self.duration_to_width(dur_tick);
            if w < 1.0 {
                continue;
            }
            // Cell border
            frame.stroke(
                &Path::rounded_rectangle(
                    Point::new(x0 + 1.0, 3.0),
                    Size::new((w - 2.0).max(2.0), VR_CHORD_STRIP_HEIGHT - 6.0),
                    3.0.into(),
                ),
                Stroke::default()
                    .with_color(Color { a: 0.45, ..theme::WARM })
                    .with_width(1.0),
            );
            // Symbol — root + quality. Italic serif for the symbol.
            let label = chord_label(&c.chord);
            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(x0 + 8.0, 6.0),
                color: theme::WARM,
                size: 13.0.into(),
                font: theme::SERIF_ITALIC_FONT,
                ..canvas::Text::default()
            });
        }
    }

    /// Phoneme strip — per-note ARPAbet breakdown. Walks the notes with
    /// the same cursor `effective_syllables` uses, so phonemes shift in
    /// lockstep with syllables when slurs are added or removed:
    ///
    /// * Slur notes (`"+"`) show only the previous syllable's vowel
    ///   (held over) instead of a fresh phoneme group, matching what
    ///   the SVS pipeline will actually sing.
    /// * Non-slur notes consume the next phoneme group from
    ///   `g2p::phonemes_for_draft`.
    fn draw_phoneme_strip(
        &self,
        frame: &mut Frame,
        grid_x: f32,
        grid_w: f32,
        phonemes: &[Vec<&'static str>],
    ) {
        frame.fill_rectangle(
            Point::new(grid_x, VR_CHORD_STRIP_HEIGHT),
            Size::new(grid_w, VR_PHONEME_STRIP_HEIGHT),
            theme::BG_1,
        );
        // Section label on the left edge.
        frame.fill_text(canvas::Text {
            content: "PHN".to_string(),
            position: Point::new(6.0, VR_CHORD_STRIP_HEIGHT + 5.0),
            color: theme::TEXT_3,
            size: 8.5.into(),
            font: theme::UI_FONT_SEMIBOLD,
            ..canvas::Text::default()
        });
        let strip_y = VR_CHORD_STRIP_HEIGHT + 4.0;

        if self.clip.notes.is_empty() || phonemes.is_empty() {
            frame.fill_text(canvas::Text {
                content: "(no phonemes \u{2014} generate from the right rail)".to_string(),
                position: Point::new(grid_x + 8.0, strip_y + 1.0),
                color: theme::TEXT_3,
                size: 10.5.into(),
                font: theme::SERIF_ITALIC_FONT,
                ..canvas::Text::default()
            });
            return;
        }

        let mut cursor: usize = 0;
        // Track the last non-slur syllable's vowel so slur notes can
        // visualise the held phoneme.
        let mut last_vowel: Option<&'static str> = None;
        for (i, n) in self.clip.notes.iter().enumerate() {
            let entry = self.lyrics.get(i).map(|s| s.trim()).unwrap_or("");
            let is_slur = entry == "+" || entry == "-";

            let display: String = if is_slur {
                last_vowel
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "+".to_string())
            } else {
                // Non-slur: consume the next phoneme list from the
                // draft. Stops cleanly once the list runs out so a
                // user-extended phrase doesn't loop the phonemes.
                let group = phonemes.get(cursor);
                cursor += 1;
                let Some(group) = group else { continue };
                if group.is_empty() {
                    continue;
                }
                // Cache the *last* non-consonant phoneme — that's the
                // vowel the SVS will sustain into a following slur.
                if let Some(v) = group.iter().rev().find(|p| !g2p::is_consonant(p)) {
                    last_vowel = Some(*v);
                } else if let Some(v) = group.last() {
                    last_vowel = Some(*v);
                }
                group.join(" ")
            };
            if display.is_empty() {
                continue;
            }

            let x = grid_x + self.tick_to_x(n.start_tick);
            let nw = self.duration_to_width(n.duration_ticks).max(8.0);
            if x > grid_x + grid_w {
                break;
            }
            let pill_w = nw.clamp(14.0, 72.0);
            // Slur pills read lighter — same visual hierarchy as the
            // notes themselves (slur = continuation, not new attack).
            let pill_alpha = if is_slur { 0.06 } else { 0.10 };
            frame.fill(
                &Path::rounded_rectangle(
                    Point::new(x + 1.0, strip_y - 1.0),
                    Size::new(pill_w - 2.0, VR_PHONEME_STRIP_HEIGHT - 6.0),
                    3.0.into(),
                ),
                Color { a: pill_alpha, ..theme::WARM },
            );
            let text_color = if is_slur {
                Color { a: 0.65, ..theme::WARM }
            } else {
                theme::WARM
            };
            frame.fill_text(canvas::Text {
                content: display,
                position: Point::new(x + 4.0, strip_y + 1.0),
                color: text_color,
                size: 9.5.into(),
                font: theme::MONO_FONT,
                ..canvas::Text::default()
            });
        }
    }
}

/// Format the chord as a short label (e.g. "Bm", "F#7"). Uses the
/// `Display` impls already published by `resonance-music-theory`.
fn chord_label(chord: &resonance_music_theory::Chord) -> String {
    format!("{}{}", chord.root, chord.quality)
}

/// Per-note labels for the vocal roll. The side-table entries are
/// interpreted as *annotations*, not absolute labels:
///
/// * `""` (empty) — pull the next syllable from the auto-syllabified
///   draft. This is the default for every note.
/// * `"+"` (or `"-"`) — slur. The note inherits the previous syllable
///   and does *not* advance the draft cursor, so every syllable after
///   the slur shifts one note to the right.
/// * any other string — explicit per-note lyric override (used for
///   manual edits; the cursor still advances past this note).
///
/// Walking the notes with a cursor like this means adding or removing
/// a slur in the middle of a phrase moves the trailing syllables in
/// lockstep without rewriting the side-table — the user sees the
/// rest of the lyrics slide along.
fn effective_syllables(c: &VocalRollCanvas<'_>) -> Vec<String> {
    let note_count = c.clip.notes.len();
    let pool = draft_syllable_pool(c.params);

    let mut out: Vec<String> = Vec::with_capacity(note_count);
    let mut cursor: usize = 0;
    for i in 0..note_count {
        let entry = c.lyrics.get(i).map(|s| s.trim()).unwrap_or("");
        if entry == "+" || entry == "-" {
            out.push("+".to_string());
        } else if !entry.is_empty() {
            out.push(entry.to_string());
            cursor += 1;
        } else {
            let label = pool.get(cursor).cloned().unwrap_or_default();
            out.push(label);
            cursor += 1;
        }
    }
    out
}

/// Flatten the lane's `params.draft` into a per-syllable pool. Same
/// auto-syllabification path the SVS pipeline + g2p use, so the
/// vocal roll's labels stay aligned with what the model will sing.
fn draft_syllable_pool(params: &VocalParams) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in &params.draft {
        let syllabified = g2p::auto_syllabify_text(&line.text);
        for word in syllabified.split_whitespace() {
            for part in word.split('\u{00B7}') {
                let t = part.trim();
                if !t.is_empty() {
                    out.push(t.to_string());
                }
            }
        }
    }
    out
}

/// `true` when the lyric at `i` is the OpenUtau slur marker (`"+"` or
/// `"-"`). Returns `false` for empty or out-of-range indices.
fn is_slur_note(lyrics: &[String], i: usize) -> bool {
    lyrics
        .get(i)
        .map(|l| {
            let s = l.trim();
            s == "+" || s == "-"
        })
        .unwrap_or(false)
}
