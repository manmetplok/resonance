/// Piano roll MIDI editor canvas for the Resonance DAW.
use iced::widget::canvas;
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use std::collections::BTreeSet;

use crate::message::*;
use crate::view::piano_roll::{
    self, hit_test_note, is_black_key, NoteEdge, NoteStyle, PianoRollLayout, PianoRollViewport,
    NOTE_COUNT,
};
use crate::state::MidiClipState;
use crate::theme;

use resonance_audio::quantize::{
    quantize_notes, BarRuler, Division, GridModifier, QuantizeMode,
};
use resonance_audio::types::{MidiNote, TempoMap, TrackId, TICKS_PER_QUARTER_NOTE};

/// Width of the piano keyboard area on the left side of the editor.
pub const KEYBOARD_WIDTH: f32 = 50.0;
/// Height of the velocity lane at the bottom of the editor.
const VELOCITY_LANE_HEIGHT: f32 = 40.0;
/// Default velocity for newly created notes.
const DEFAULT_VELOCITY: f32 = 0.8;

/// The active Quantize-panel settings, projected into the piano roll so it
/// can draw the live quantize grid and the non-destructive "ghost" target
/// preview for the current selection (todo #396). A plain `Copy` snapshot of
/// [`MidiQuantizePanelState`](crate::state::MidiQuantizePanelState) — the
/// canvas never mutates it.
#[derive(Debug, Clone, Copy)]
pub struct QuantizePreview {
    /// Grid the notes snap to (carries triplet / dotted feel).
    pub division: Division,
    /// Blend toward the grid, `0.0..=1.0`.
    pub strength: f32,
    /// Swing applied to odd grid steps, `0.0..=1.0`.
    pub swing: f32,
    /// Whether starts only, or starts + length, are quantized.
    pub mode: QuantizeMode,
    /// Snap note-offs to the grid as well as note-ons.
    pub quantize_ends: bool,
    /// Apply the strength blend iteratively (soft quantize).
    pub iterative: bool,
}

/// Data passed to the piano roll canvas for rendering.
#[derive(Debug)]
pub struct PianoRollCanvas<'a> {
    pub clip: &'a MidiClipState,
    pub track_id: TrackId,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub snap_ticks: u64,
    pub selected_notes: &'a BTreeSet<usize>,
    pub time_sig_num: u8,
    /// Live Quantize-panel settings driving the grid + ghost overlay.
    pub quantize: QuantizePreview,
    /// Project tempo / signature map, anchoring the quantize grid lines and
    /// the ghost-target snap so odd meters land correctly.
    pub tempo_map: &'a TempoMap,
}

/// Interaction mode being tracked during a drag operation.
#[derive(Debug, Clone)]
enum DragMode {
    /// Moving a note: (note_index, tick_offset_from_cursor).
    MoveNote {
        note_index: usize,
        start_tick_offset: i64,
    },
    /// Resizing a note from its right edge.
    ResizeNote { note_index: usize, anchor_tick: u64 },
}

/// Minimum cursor travel (in pixels) before a left-press on empty grid is
/// treated as a rubber-band marquee rather than a click that creates a
/// note. Keeps a slightly-shaky click from drawing a stray selection.
const MARQUEE_THRESHOLD_PX: f32 = 4.0;

/// A left-press that started on empty grid space. It resolves on release:
/// a negligible drag is a click (creates a note at `create_*`), a longer
/// drag is a marquee selection over the swept rectangle.
#[derive(Debug, Clone, Copy)]
struct EmptyDrag {
    /// Canvas-local press point.
    origin: Point,
    /// Canvas-local current cursor point.
    current: Point,
    /// The note + snapped start tick to create if this turns out to be a
    /// plain click rather than a marquee drag.
    create_note: u8,
    create_tick: u64,
}

impl EmptyDrag {
    /// Whether the cursor has travelled far enough to count as a marquee.
    fn is_marquee(&self) -> bool {
        (self.current.x - self.origin.x).abs() >= MARQUEE_THRESHOLD_PX
            || (self.current.y - self.origin.y).abs() >= MARQUEE_THRESHOLD_PX
    }

    /// The swept (normalised) rectangle, in canvas-local coordinates.
    fn rect(&self) -> Rectangle {
        piano_roll::rect_from_points(self.origin, self.current)
    }
}

/// Local state for the piano roll canvas, tracking drags and previews.
#[derive(Debug, Default)]
pub struct PianoRollState {
    drag: Option<DragMode>,
    previewing_note: Option<u8>,
    /// Active empty-grid press: either a pending note-create or an
    /// in-progress rubber-band marquee (see [`EmptyDrag`]).
    empty_drag: Option<EmptyDrag>,
    /// Latest keyboard modifier state, tracked from `ModifiersChanged`
    /// events so mouse presses can tell shift/ctrl-click from a plain
    /// click (mouse events don't carry modifiers on their own).
    modifiers: iced::keyboard::Modifiers,
    /// Cached drawn geometry — invalidated only when the fingerprint of
    /// the inputs (notes / scroll / zoom / selection / clip identity)
    /// changes. Without this the piano roll redrew on every hover and
    /// engine-event tick, which made window resize feel particularly
    /// chunky because every paint had to re-rasterize ~100 note rects.
    cache: canvas::Cache,
    cache_fingerprint: std::cell::Cell<PianoRollFingerprint>,
}

/// Minimal projection of the piano roll's inputs into a comparable
/// value. The draw routine asks for the current fingerprint, compares
/// it with what was used for the cached geometry, and only re-runs the
/// drawing closure when something visible has actually changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PianoRollFingerprint {
    pub clip_id: u64,
    pub notes_len: usize,
    /// Hash of the (note, start_tick, duration_ticks, velocity) tuples
    /// so an edit inside the clip invalidates the cache even when
    /// `notes_len` doesn't change.
    pub notes_hash: u64,
    pub scroll_x_bits: u32,
    pub scroll_y_bits: u32,
    pub zoom_x_bits: u32,
    pub zoom_y_bits: u32,
    pub snap_ticks: u64,
    pub selected_notes_hash: u64,
    pub time_sig_num: u8,
    pub drag_active: bool,
    pub preview_note: Option<u8>,
    /// Hash of the active quantize settings (grid / strength / swing /
    /// mode / ends / iterative) so the cached grid + ghost overlay
    /// invalidate the moment the user adjusts the Quantize panel.
    pub quantize_hash: u64,
}

/// Hash the selection set into a single comparable value for the draw
/// cache fingerprint. `BTreeSet` iterates in sorted order, so equal
/// selections always hash equally.
fn hash_selection(set: &BTreeSet<usize>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    set.len().hash(&mut h);
    for i in set {
        i.hash(&mut h);
    }
    h.finish()
}

/// Small stable discriminant for a grid modifier, used in the cache
/// fingerprint (the enum doesn't derive `Hash`).
fn modifier_code(m: GridModifier) -> u8 {
    match m {
        GridModifier::Straight => 0,
        GridModifier::Triplet => 1,
        GridModifier::Dotted => 2,
    }
}

/// Swing delay (ticks) applied to odd grid steps for step size `g`,
/// mirroring `resonance_audio::quantize`'s private `swing_delay` so the
/// drawn grid lines land exactly where the ghost preview snaps notes.
fn swing_delay(g: u64, swing: f32) -> u64 {
    let s = swing.clamp(0.0, 1.0) as f64;
    (s * g as f64 / 2.0).round() as u64
}

/// Local tick offsets (within a bar of `bar_len` ticks) of every quantize
/// grid line for step size `step_ticks`, swung on odd steps by `swing`.
///
/// The returned offsets start at the downbeat (`0`) and never exceed
/// `bar_len`; odd-indexed steps are delayed by [`swing_delay`] so a triplet
/// / swing feel reads as the off-beats sliding later. This is the single
/// source of grid-line geometry — the renderer walks it and its unit tests
/// assert it — so the drawn lines and the ghost snap can't drift apart.
pub fn quantize_grid_steps(step_ticks: u64, bar_len: u64, swing: f32) -> Vec<u64> {
    if step_ticks == 0 || bar_len == 0 {
        return Vec::new();
    }
    let delay = swing_delay(step_ticks, swing);
    let mut out = Vec::new();
    let mut k = 0u64;
    loop {
        let base = k * step_ticks;
        if base >= bar_len {
            break;
        }
        let local = if k % 2 == 1 {
            (base + delay).min(bar_len)
        } else {
            base
        };
        out.push(local);
        k += 1;
    }
    out
}

/// Quantized target positions for `selection`, anchored at clip tick 0 so
/// the ghost preview lands exactly on the grid drawn by
/// [`quantize_grid_steps`]. A thin wrapper over the pure
/// [`quantize_notes`] used by the engine's Apply, so the preview and the
/// committed result agree note-for-note (modulo the clip-start anchoring
/// the visible grid already assumes).
pub fn ghost_targets(
    notes: &[MidiNote],
    selection: &[usize],
    q: &QuantizePreview,
    tempo: &TempoMap,
) -> Vec<MidiNote> {
    quantize_notes(
        notes,
        selection,
        q.division,
        q.strength,
        q.swing,
        q.mode,
        q.quantize_ends,
        q.iterative,
        tempo,
        0,
    )
}

impl canvas::Program<Message> for PianoRollCanvas<'_> {
    type State = PianoRollState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let layout = self.layout(bounds);
        let viewport = self.viewport();
        let grid_x = layout.grid_x();
        let grid_h = layout.grid_h;

        match event {
            // --- Scroll ---
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                // Only handle wheel events when the cursor is actually over the
                // piano roll — otherwise scrolling the arrangement would also
                // scroll this editor.
                cursor.position_in(bounds)?;
                // Horizontal scroll is handled by the outer `Scrollable`
                // that wraps this canvas now (see `view_midi_editor_panel`).
                // Returning `Ignored` lets the event bubble up. Vertical
                // pitch scroll stays inside the canvas because the
                // keyboard column needs to scroll in lockstep with the
                // note rows.
                match delta {
                    mouse::ScrollDelta::Lines { x, y } => {
                        if x.abs() > f32::EPSILON {
                            return None;
                        }
                        return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::ScrollY(-y * 30.0))).and_capture());
                    }
                    mouse::ScrollDelta::Pixels { x, y } => {
                        if x.abs() > f32::EPSILON {
                            return None;
                        }
                        return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::ScrollY(-y))).and_capture());
                    }
                }
            }

            // --- Mouse press ---
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    // Piano keyboard area: preview note
                    if pos.x < grid_x && pos.y < grid_h {
                        let note = viewport.y_local_to_note(pos.y);
                        state.previewing_note = Some(note);
                        return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::PreviewNote(
                                self.track_id,
                                note,
                            ))).and_capture());
                    }

                    // Velocity lane: not interactive for now (future: drag velocity bars)
                    if pos.y >= grid_h {
                        return None;
                    }

                    // Note grid area
                    if pos.x >= grid_x {
                        let click_tick = viewport.x_local_to_tick(pos.x - grid_x);
                        let click_note = viewport.y_local_to_note(pos.y);
                        // Shift/Ctrl held → additive selection edit, not a drag.
                        let additive = state.modifiers.shift() || state.modifiers.command();

                        // Check if clicking on an existing note
                        for (i, n) in self.clip.notes.iter().enumerate() {
                            let rect = self.note_rect(&layout, &viewport, n);
                            if let Some(edge) = hit_test_note(rect, pos) {
                                // Shift/Ctrl-click toggles the note's membership
                                // and never starts a drag — it's purely a
                                // selection edit.
                                if additive {
                                    return Some(canvas::Action::publish(Message::MidiEditor(
                                        MidiEditorMessage::ToggleNoteSelection { note_index: i },
                                    )).and_capture());
                                }
                                state.drag = Some(match edge {
                                    NoteEdge::ResizeRight => DragMode::ResizeNote {
                                        note_index: i,
                                        anchor_tick: n.start_tick,
                                    },
                                    NoteEdge::Body => {
                                        let tick_offset =
                                            n.start_tick as i64 - click_tick as i64;
                                        DragMode::MoveNote {
                                            note_index: i,
                                            start_tick_offset: tick_offset,
                                        }
                                    }
                                });
                                return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::SelectNote {
                                        note_index: Some(i),
                                    })).and_capture());
                            }
                        }

                        // Empty space: defer the decision to release. A
                        // negligible drag is a click that creates a note; a
                        // longer drag is a rubber-band marquee selection.
                        let snapped = self.snap(click_tick);
                        state.empty_drag = Some(EmptyDrag {
                            origin: pos,
                            current: pos,
                            create_note: click_note,
                            create_tick: snapped,
                        });
                        return Some(canvas::Action::capture());
                    }
                }
            }

            // --- Right-click: remove selected note ---
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    if pos.x >= grid_x && pos.y < grid_h {
                        for (i, n) in self.clip.notes.iter().enumerate() {
                            let rect = self.note_rect(&layout, &viewport, n);
                            if hit_test_note(rect, pos).is_some() {
                                return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::RemoveNote {
                                        clip_id: self.clip.id,
                                        note_index: i,
                                    })).and_capture());
                            }
                        }
                    }
                }
            }

            // --- Mouse move (drag) ---
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    // Extend an in-progress empty-grid press (marquee). The
                    // overlay repaints on the next periodic UI tick.
                    if let Some(ref mut ed) = state.empty_drag {
                        ed.current = pos;
                        return Some(canvas::Action::capture());
                    }
                    match &state.drag {
                        Some(DragMode::MoveNote {
                            note_index,
                            start_tick_offset,
                            ..
                        }) if pos.x >= grid_x && pos.y < grid_h => {
                            let tick = viewport.x_local_to_tick(pos.x - grid_x);
                            let raw_tick = (tick as i64 + start_tick_offset).max(0) as u64;
                            let snapped_tick = self.snap(raw_tick);
                            let note = viewport.y_local_to_note(pos.y);
                            return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::MoveNote {
                                    clip_id: self.clip.id,
                                    note_index: *note_index,
                                    new_start_tick: snapped_tick,
                                    new_note: note,
                                })).and_capture());
                        }
                        Some(DragMode::ResizeNote {
                            note_index,
                            anchor_tick,
                        }) if pos.x >= grid_x => {
                            let tick = viewport.x_local_to_tick(pos.x - grid_x);
                            let snapped = self.snap(tick);
                            let new_dur =
                                snapped.saturating_sub(*anchor_tick).max(self.snap_ticks);
                            return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::ResizeNote {
                                    clip_id: self.clip.id,
                                    note_index: *note_index,
                                    new_duration_ticks: new_dur,
                                })).and_capture());
                        }
                        Some(_) | None => {}
                    }
                }
            }

            // --- Mouse release ---
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag = None;
                // Resolve an empty-grid press.
                if let Some(ed) = state.empty_drag.take() {
                    if ed.is_marquee() {
                        let indices = piano_roll::notes_in_marquee(
                            &self.clip.notes,
                            &layout,
                            &viewport,
                            ed.rect(),
                        );
                        let additive = state.modifiers.shift();
                        return Some(canvas::Action::publish(Message::MidiEditor(
                            MidiEditorMessage::SelectNotesInRect { indices, additive },
                        )).and_capture());
                    } else if !self.selected_notes.is_empty() {
                        // Plain click on empty space with an active selection
                        // clears it rather than creating a note.
                        return Some(canvas::Action::publish(Message::MidiEditor(
                            MidiEditorMessage::ClearNoteSelection,
                        )).and_capture());
                    } else {
                        // Nothing selected: a plain empty click creates a note.
                        return Some(canvas::Action::publish(Message::MidiEditor(
                            MidiEditorMessage::AddNote {
                                clip_id: self.clip.id,
                                note: ed.create_note,
                                start_tick: ed.create_tick,
                                duration_ticks: self.snap_ticks,
                                velocity: DEFAULT_VELOCITY,
                            },
                        )).and_capture());
                    }
                }
                if let Some(note) = state.previewing_note.take() {
                    return Some(canvas::Action::publish(Message::MidiEditor(MidiEditorMessage::StopPreview(
                            self.track_id,
                            note,
                        ))).and_capture());
                }
            }

            // --- Delete key: remove selected note ---
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Delete),
                ..
            })
            | iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Backspace),
                ..
            }) => {
                if !self.selected_notes.is_empty() {
                    return Some(canvas::Action::publish(Message::MidiEditor(
                        MidiEditorMessage::RemoveSelectedNotes {
                            clip_id: self.clip.id,
                        },
                    )).and_capture());
                }
            }

            // --- Ctrl/Cmd+Shift+A: select the notes currently in view ---
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Character(ref c),
                modifiers,
                ..
            }) if modifiers.command()
                && modifiers.shift()
                && c.as_str().eq_ignore_ascii_case("a") =>
            {
                let view_rect = Rectangle {
                    x: grid_x,
                    y: 0.0,
                    width: (bounds.width - grid_x).max(0.0),
                    height: grid_h,
                };
                let indices =
                    piano_roll::notes_in_marquee(&self.clip.notes, &layout, &viewport, view_rect);
                return Some(canvas::Action::publish(Message::MidiEditor(
                    MidiEditorMessage::SelectNotesInRect {
                        indices,
                        additive: false,
                    },
                )).and_capture());
            }

            // --- Ctrl/Cmd+A: select every note in the clip ---
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Character(ref c),
                modifiers,
                ..
            }) if modifiers.command() && c.as_str().eq_ignore_ascii_case("a") => {
                return Some(canvas::Action::publish(Message::MidiEditor(
                    MidiEditorMessage::SelectAllNotes,
                )).and_capture());
            }

            // --- Track modifier state for shift/ctrl-aware mouse clicks ---
            iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(mods)) => {
                state.modifiers = *mods;
            }

            _ => {}
        }
        None
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let fp = self.fingerprint(state);
        if state.cache_fingerprint.get() != fp {
            state.cache.clear();
            state.cache_fingerprint.set(fp);
        }
        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            self.draw_into(frame, bounds);
        });
        // The rubber-band marquee is drawn live, outside the cached note
        // layer, so dragging it doesn't invalidate ~100 cached note rects
        // every frame; it repaints with the periodic UI tick.
        if let Some(ed) = state.empty_drag {
            if ed.is_marquee() {
                let rect = ed.rect();
                let mut overlay = canvas::Frame::new(renderer, bounds.size());
                overlay.fill_rectangle(
                    Point::new(rect.x, rect.y),
                    Size::new(rect.width, rect.height),
                    Color {
                        a: 0.15,
                        ..theme::ACCENT
                    },
                );
                overlay.stroke(
                    &canvas::Path::rectangle(
                        Point::new(rect.x, rect.y),
                        Size::new(rect.width, rect.height),
                    ),
                    canvas::Stroke::default()
                        .with_color(theme::ACCENT)
                        .with_width(1.0),
                );
                return vec![geometry, overlay.into_geometry()];
            }
        }
        vec![geometry]
    }
}

impl PianoRollCanvas<'_> {
    /// Layout for the bottom-panel piano roll: keyboard on the left,
    /// no toolbar, velocity lane below the grid.
    fn layout(&self, bounds: Rectangle) -> PianoRollLayout {
        PianoRollLayout {
            keyboard_w: KEYBOARD_WIDTH,
            grid_top: 0.0,
            grid_h: bounds.height - VELOCITY_LANE_HEIGHT,
        }
    }

    fn viewport(&self) -> PianoRollViewport {
        PianoRollViewport {
            zoom_x: self.zoom_x,
            zoom_y: self.zoom_y,
            scroll_x: self.scroll_x,
            scroll_y: self.scroll_y,
        }
    }

    /// Pixel rectangle for `note`, in canvas-local coordinates.
    fn note_rect(
        &self,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
        note: &resonance_audio::types::MidiNote,
    ) -> Rectangle {
        Rectangle {
            x: layout.grid_x() + viewport.tick_to_x_local(note.start_tick),
            y: layout.grid_top + viewport.note_to_y_local(note.note),
            width: viewport.duration_to_w(note.duration_ticks),
            height: viewport.zoom_y,
        }
    }

    /// Snap a tick value to the nearest grid position.
    fn snap(&self, tick: u64) -> u64 {
        if self.snap_ticks == 0 {
            return tick;
        }
        let half = self.snap_ticks / 2;
        ((tick + half) / self.snap_ticks) * self.snap_ticks
    }

    /// Hash of the inputs that affect the drawn geometry. Excludes
    /// `bounds.size()` because the cache invalidates on size change
    /// automatically (via `canvas::Cache::draw`), so adding it here
    /// would double the work during a resize.
    fn fingerprint(&self, state: &PianoRollState) -> PianoRollFingerprint {
        use std::hash::{Hash, Hasher};
        let mut nh = std::collections::hash_map::DefaultHasher::new();
        for n in &self.clip.notes {
            n.note.hash(&mut nh);
            n.start_tick.hash(&mut nh);
            n.duration_ticks.hash(&mut nh);
            n.velocity.to_bits().hash(&mut nh);
        }
        PianoRollFingerprint {
            clip_id: self.clip.id,
            notes_len: self.clip.notes.len(),
            notes_hash: nh.finish(),
            scroll_x_bits: self.scroll_x.to_bits(),
            scroll_y_bits: self.scroll_y.to_bits(),
            zoom_x_bits: self.zoom_x.to_bits(),
            zoom_y_bits: self.zoom_y.to_bits(),
            snap_ticks: self.snap_ticks,
            selected_notes_hash: hash_selection(self.selected_notes),
            time_sig_num: self.time_sig_num,
            drag_active: state.drag.is_some(),
            preview_note: state.previewing_note,
            quantize_hash: self.quantize_hash(),
        }
    }

    /// Fold the active quantize settings into one comparable value for the
    /// draw-cache fingerprint. The quantize enums don't derive `Hash`, so
    /// they're folded in via their tick size / a small discriminant rather
    /// than `Hash`. Selection already lives in `selected_notes_hash`, so it
    /// isn't repeated here.
    fn quantize_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let q = &self.quantize;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        // `division.ticks()` is distinct per (value, modifier) across the
        // supported grid set, but fold a modifier code in too for safety.
        q.division.ticks().hash(&mut h);
        modifier_code(q.division.modifier).hash(&mut h);
        q.strength.to_bits().hash(&mut h);
        q.swing.to_bits().hash(&mut h);
        matches!(q.mode, QuantizeMode::StartAndLength).hash(&mut h);
        q.quantize_ends.hash(&mut h);
        q.iterative.hash(&mut h);
        h.finish()
    }

    fn draw_into(&self, frame: &mut canvas::Frame, bounds: Rectangle) {
        let layout = self.layout(bounds);
        let viewport = self.viewport();
        let grid_x = layout.grid_x();
        let grid_w = bounds.width - grid_x;
        let grid_h = layout.grid_h;

        // --- Background ---
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::BG);

        // --- Note row backgrounds ---
        self.draw_note_rows(frame, &viewport, grid_x, grid_w, grid_h);

        // --- Grid lines ---
        self.draw_grid_lines(frame, &viewport, grid_x, grid_w, grid_h);

        // --- Active quantize grid (triplet / dotted / swing) ---
        self.draw_quantize_grid(frame, &viewport, grid_x, grid_w, grid_h);

        // --- Notes ---
        self.draw_notes(frame, &layout, &viewport);

        // --- Ghost-target preview for the current selection ---
        self.draw_ghost_notes(frame, &layout, &viewport);

        // --- Piano keyboard ---
        piano_roll::draw_keyboard(frame, &layout, &viewport);

        // --- Velocity lane ---
        self.draw_velocity_lane(frame, &viewport, grid_x, grid_w, grid_h, bounds.height);

        // --- Separator lines ---
        // Vertical separator between keyboard and grid
        frame.fill_rectangle(
            Point::new(grid_x, 0.0),
            Size::new(1.0, grid_h),
            theme::SEPARATOR,
        );
        // Horizontal separator between grid and velocity lane
        frame.fill_rectangle(
            Point::new(0.0, grid_h),
            Size::new(bounds.width, 1.0),
            theme::SEPARATOR,
        );
    }

    /// Draw alternating row backgrounds for each semitone.
    fn draw_note_rows(
        &self,
        frame: &mut canvas::Frame,
        viewport: &PianoRollViewport,
        grid_x: f32,
        grid_w: f32,
        grid_h: f32,
    ) {
        // Backdrop is BG_2; only black-key rows darken to BG_1. White
        // keys reuse the backdrop so the row striping reads softly.
        frame.fill_rectangle(
            Point::new(grid_x, 0.0),
            Size::new(grid_w, grid_h),
            theme::BG_2,
        );
        for midi_note in 0..NOTE_COUNT {
            let y = viewport.note_to_y_local(midi_note);
            let h = viewport.zoom_y;

            if y + h < 0.0 || y > grid_h {
                continue;
            }

            if is_black_key(midi_note) {
                frame.fill_rectangle(Point::new(grid_x, y), Size::new(grid_w, h), theme::BG_1);
            }

            if midi_note % 12 == 0 {
                frame.fill_rectangle(
                    Point::new(grid_x, y + h - 1.0),
                    Size::new(grid_w, 1.0),
                    theme::LINE_2,
                );
            }
        }
    }

    /// Draw vertical grid lines at beat and bar boundaries.
    fn draw_grid_lines(
        &self,
        frame: &mut canvas::Frame,
        viewport: &PianoRollViewport,
        grid_x: f32,
        grid_w: f32,
        grid_h: f32,
    ) {
        let ticks_per_beat = TICKS_PER_QUARTER_NOTE;
        let ticks_per_bar = TICKS_PER_QUARTER_NOTE * self.time_sig_num as u64;
        let pixels_per_beat = ticks_per_beat as f32 * viewport.zoom_x;

        // Determine visible tick range
        let start_tick = (viewport.scroll_x / viewport.zoom_x).max(0.0) as u64;
        let end_tick = ((viewport.scroll_x + grid_w) / viewport.zoom_x) as u64 + ticks_per_beat;

        // Draw beat lines
        if pixels_per_beat >= 8.0 {
            let first_beat = start_tick / ticks_per_beat;
            let last_beat = end_tick / ticks_per_beat + 1;

            for beat_idx in first_beat..=last_beat {
                let tick = beat_idx * ticks_per_beat;
                let x = grid_x + viewport.tick_to_x_local(tick);

                if x < grid_x || x > grid_x + grid_w {
                    continue;
                }

                let is_bar = tick.is_multiple_of(ticks_per_bar);
                let color = if is_bar {
                    theme::BAR_LINE
                } else {
                    theme::BEAT_LINE
                };

                frame.fill_rectangle(Point::new(x, 0.0), Size::new(1.0, grid_h), color);
            }
        }

        // Draw subdivision lines (16th notes) if zoomed in enough
        let snap_px = self.snap_ticks as f32 * viewport.zoom_x;
        if snap_px >= 8.0 && self.snap_ticks < ticks_per_beat {
            let first = start_tick / self.snap_ticks;
            let last = end_tick / self.snap_ticks + 1;
            for idx in first..=last {
                let tick = idx * self.snap_ticks;
                if tick.is_multiple_of(ticks_per_beat) {
                    continue; // already drawn as beat/bar line
                }
                let x = grid_x + viewport.tick_to_x_local(tick);
                if x < grid_x || x > grid_x + grid_w {
                    continue;
                }
                frame.fill_rectangle(
                    Point::new(x, 0.0),
                    Size::new(1.0, grid_h),
                    Color {
                        a: 0.5,
                        ..theme::LINE_2
                    },
                );
            }
        }
    }

    /// Draw the active quantize grid: vertical lines at every step of the
    /// selected division, swung on odd steps, anchored to bars via the
    /// project tempo map so triplet / dotted / swing feels read correctly.
    ///
    /// These sit on top of the editor's plain beat / bar / snap lines in a
    /// faint accent tint so the user can see exactly where Apply will pull
    /// notes — and they shift live as the grid / swing change because the
    /// whole geometry is keyed on `quantize_hash` in the draw cache.
    fn draw_quantize_grid(
        &self,
        frame: &mut canvas::Frame,
        viewport: &PianoRollViewport,
        grid_x: f32,
        grid_w: f32,
        grid_h: f32,
    ) {
        let g = self.quantize.division.ticks();
        let step_px = g as f32 * viewport.zoom_x;
        // Too dense to read (or zero-width) — skip rather than smear the grid.
        if step_px < 5.0 {
            return;
        }

        let swung = self.quantize.swing > f32::EPSILON;

        // Visible absolute-tick span (clip-relative ticks == absolute here:
        // the grid and the ghost both anchor at clip tick 0, matching how
        // the editor already draws its bar / beat lines).
        let start_tick = (viewport.scroll_x / viewport.zoom_x).max(0.0) as u64;
        let end_tick = ((viewport.scroll_x + grid_w) / viewport.zoom_x) as u64 + g;

        let ruler = BarRuler::new(self.tempo_map);
        let (mut bar_start, mut bar_len) = ruler.bar_at(start_tick);

        // Walk bar by bar so a mid-project signature change re-anchors the
        // grid; cap the walk so a degenerate tempo map can't spin forever.
        let mut guard = 0;
        while bar_start < end_tick && guard < 4096 {
            guard += 1;
            for (k, local) in quantize_grid_steps(g, bar_len, self.quantize.swing)
                .into_iter()
                .enumerate()
            {
                // Downbeats are already a bold bar line; don't double-draw.
                if local == 0 {
                    continue;
                }
                let x = grid_x + viewport.tick_to_x_local(bar_start + local);
                if x < grid_x || x > grid_x + grid_w {
                    continue;
                }
                // Swung off-beats (odd steps) read a touch brighter so the
                // swing offset is visible against the straight steps.
                let alpha = if swung && k % 2 == 1 { 0.42 } else { 0.26 };
                frame.fill_rectangle(
                    Point::new(x, 0.0),
                    Size::new(1.0, grid_h),
                    Color {
                        a: alpha,
                        ..theme::ACCENT
                    },
                );
            }
            bar_start += bar_len;
            bar_len = ruler.bar_at(bar_start).1;
        }
    }

    /// Notes Apply would land on, for the current selection and quantize
    /// settings. Returns `None` when nothing is selected (the grid alone
    /// previews the target then) — the ghost is scoped to the selection so
    /// it stays readable, matching the panel's "selected notes" wording.
    fn ghost_notes(&self) -> Option<Vec<MidiNote>> {
        if self.selected_notes.is_empty() || self.quantize.strength <= f32::EPSILON {
            return None;
        }
        let selection: Vec<usize> = self.selected_notes.iter().copied().collect();
        Some(ghost_targets(
            &self.clip.notes,
            &selection,
            &self.quantize,
            self.tempo_map,
        ))
    }

    /// Draw the non-destructive ghost preview: a dashed, translucent warm
    /// rectangle at each selected note's quantized target, with a faint
    /// connector from its current position so the move reads at a glance.
    /// Targets that don't actually move (already on grid) are skipped.
    fn draw_ghost_notes(
        &self,
        frame: &mut canvas::Frame,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
    ) {
        let Some(ghosts) = self.ghost_notes() else {
            return;
        };
        const DASH: canvas::LineDash<'static> = canvas::LineDash {
            segments: &[3.0, 2.0],
            offset: 0,
        };
        for &i in self.selected_notes {
            let Some(orig) = self.clip.notes.get(i) else {
                continue;
            };
            let Some(ghost) = ghosts.get(i) else { continue };
            // No movement → nothing to preview.
            if ghost.start_tick == orig.start_tick
                && ghost.duration_ticks == orig.duration_ticks
            {
                continue;
            }
            let rect = self.note_rect(layout, viewport, ghost);
            // Cull ghosts fully off the grid area.
            if rect.x + rect.width < layout.grid_x()
                || rect.x > layout.grid_x() + 2000.0
                || rect.y + rect.height < 0.0
                || rect.y > layout.grid_h
            {
                continue;
            }

            let orig_rect = self.note_rect(layout, viewport, orig);
            // Connector from the current note to its target (mid-height).
            let mid_y = orig_rect.y + orig_rect.height * 0.5;
            frame.stroke(
                &canvas::Path::line(
                    Point::new(orig_rect.x, mid_y),
                    Point::new(rect.x, rect.y + rect.height * 0.5),
                ),
                canvas::Stroke {
                    line_dash: DASH,
                    ..canvas::Stroke::default()
                        .with_color(Color {
                            a: 0.5,
                            ..theme::WARM
                        })
                        .with_width(1.0)
                },
            );

            let path = canvas::Path::rounded_rectangle(
                Point::new(rect.x, rect.y),
                Size::new(rect.width.max(1.0), rect.height),
                2.0.into(),
            );
            frame.fill(
                &path,
                Color {
                    a: 0.16,
                    ..theme::WARM
                },
            );
            frame.stroke(
                &path,
                canvas::Stroke {
                    line_dash: DASH,
                    ..canvas::Stroke::default()
                        .with_color(theme::WARM)
                        .with_width(1.2)
                },
            );
        }
    }

    /// Draw MIDI note rectangles on the grid.
    fn draw_notes(
        &self,
        frame: &mut canvas::Frame,
        layout: &PianoRollLayout,
        viewport: &PianoRollViewport,
    ) {
        let grid_x = layout.grid_x();
        for (i, n) in self.clip.notes.iter().enumerate() {
            let rect = self.note_rect(layout, viewport, n);

            if rect.x + rect.width < grid_x
                || rect.x > grid_x + 2000.0
                || rect.y + rect.height < 0.0
                || rect.y > layout.grid_h
            {
                continue;
            }

            let style = if self.selected_notes.contains(&i) {
                NoteStyle::selected()
            } else {
                NoteStyle::plain()
            };
            piano_roll::draw_note(frame, rect, n.velocity, style);
        }
    }

    /// Draw the velocity lane at the bottom.
    fn draw_velocity_lane(
        &self,
        frame: &mut canvas::Frame,
        viewport: &PianoRollViewport,
        grid_x: f32,
        grid_w: f32,
        grid_h: f32,
        total_h: f32,
    ) {
        let lane_y = grid_h + 1.0;
        let lane_h = total_h - grid_h - 1.0;

        // Lane background
        frame.fill_rectangle(
            Point::new(0.0, lane_y),
            Size::new(grid_x + grid_w, lane_h),
            theme::PANEL_DARK,
        );

        // "Vel" label
        frame.fill_text(canvas::Text {
            content: "Vel".to_string(),
            position: Point::new(4.0, lane_y + 2.0),
            color: theme::TEXT_DIM,
            size: 9.0.into(),
            ..canvas::Text::default()
        });

        // Velocity bars for each note
        for (i, n) in self.clip.notes.iter().enumerate() {
            let x = grid_x + viewport.tick_to_x_local(n.start_tick);
            let w = viewport.duration_to_w(n.duration_ticks).clamp(2.0, 6.0);

            if x + w < grid_x || x > grid_x + 2000.0 {
                continue;
            }

            let bar_h = n.velocity.clamp(0.0, 1.0) * (lane_h - 4.0);
            let bar_y = lane_y + lane_h - bar_h - 2.0;

            let is_selected = self.selected_notes.contains(&i);
            let color = if is_selected {
                theme::ACCENT
            } else {
                theme::ACCENT_SOFT
            };

            frame.fill_rectangle(Point::new(x, bar_y), Size::new(w, bar_h), color);
        }
    }
}
