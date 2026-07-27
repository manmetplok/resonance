//! Shared helpers for the two piano-roll canvases (the bottom-panel
//! `midi_editor::PianoRollCanvas` and the Compose-tab
//! `view::compose::expanded_editor::ExpandedEditorCanvas`).
//!
//! The two canvases were inlined copies of each other for a long stretch:
//! identical `is_black_key`, `note_name`, grid drawing, keyboard strip,
//! and hit-test helpers, differing only in bounds, accent colour, message
//! type, and overlays (the expanded editor adds chord-context + scale
//! markers). The shared coordinate math, keyboard column, note-rectangle
//! draw, and hit testing live here; each canvas still owns its own
//! `Program` impl (event routing, message types) and the parts that
//! genuinely differ (velocity lane, beat-grid bar walking, scale row
//! highlight, toolbar overlay).

use iced::widget::canvas;
use iced::{Color, Point, Rectangle, Size};

use crate::theme;

/// Total number of MIDI note rows (0..127).
pub const NOTE_COUNT: u8 = 128;

/// Minimum width in pixels from a note's right edge that counts as a
/// "resize" grab rather than a "move" grab.
pub const RESIZE_EDGE_PX: f32 = 6.0;

pub(crate) fn is_black_key(note: u8) -> bool {
    matches!(note % 12, 1 | 3 | 6 | 8 | 10)
}

/// Human-readable note name (e.g. `"C4"`, `"F#3"`). Middle C (MIDI 60)
/// renders as `"C4"` to match the standard convention used by
/// keyboard plugins and most DAWs.
pub(crate) fn note_name(note: u8) -> String {
    resonance_music_theory::midi_note_name(note)
}

/// Snap a tick value down to the nearest multiple of `snap`. `snap == 0`
/// disables snapping (returns the tick unchanged).
pub(crate) fn snap_tick(tick: u64, snap: u64) -> u64 {
    if snap == 0 {
        return tick;
    }
    (tick / snap) * snap
}

/// Pixel layout of a piano-roll canvas. The grid occupies the rectangle
/// `(grid_x().., grid_top..)` for height `grid_h`; the keyboard column
/// occupies `(0.., grid_top..)` for width `keyboard_w`.
#[derive(Debug, Clone, Copy)]
pub struct PianoRollLayout {
    /// Width of the keyboard column on the left.
    pub keyboard_w: f32,
    /// Top of the grid in canvas-local pixels (toolbar height or 0).
    pub grid_top: f32,
    /// Height of the grid area (excluding velocity lane / toolbar).
    pub grid_h: f32,
}

impl PianoRollLayout {
    /// Left edge of the grid (right edge of the keyboard column).
    pub fn grid_x(&self) -> f32 {
        self.keyboard_w
    }
}

/// Scroll/zoom for a piano-roll canvas. `zoom_x` is in pixels-per-tick,
/// `zoom_y` is in pixels-per-semitone.
#[derive(Debug, Clone, Copy)]
pub struct PianoRollViewport {
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
}

impl PianoRollViewport {
    /// Tick → grid-local pixel x (caller adds `layout.grid_x()` to get
    /// the canvas-local x).
    pub fn tick_to_x_local(&self, tick: u64) -> f32 {
        tick as f32 * self.zoom_x - self.scroll_x
    }

    /// Grid-local pixel x → tick (caller subtracts `layout.grid_x()`
    /// before passing in).
    pub fn x_local_to_tick(&self, x_local: f32) -> u64 {
        let tick = (x_local + self.scroll_x) / self.zoom_x;
        if tick < 0.0 {
            0
        } else {
            tick as u64
        }
    }

    /// Width in pixels of a note that lasts `ticks` ticks.
    pub fn duration_to_w(&self, ticks: u64) -> f32 {
        ticks as f32 * self.zoom_x
    }

    /// MIDI note number → grid-local pixel y (caller adds
    /// `layout.grid_top` to get the canvas-local y).
    pub fn note_to_y_local(&self, note: u8) -> f32 {
        let row = (NOTE_COUNT - 1 - note) as f32;
        row * self.zoom_y - self.scroll_y
    }

    /// Grid-local pixel y → MIDI note number (caller subtracts
    /// `layout.grid_top` before passing in).
    pub fn y_local_to_note(&self, y_local: f32) -> u8 {
        let row = ((y_local + self.scroll_y) / self.zoom_y).floor() as i32;
        ((NOTE_COUNT as i32 - 1) - row).clamp(0, 127) as u8
    }
}

/// Outcome of a hit test against a note rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteEdge {
    /// Click fell on the note body — interpreted as a move drag.
    Body,
    /// Click fell within `RESIZE_EDGE_PX` of the right edge —
    /// interpreted as a resize drag.
    ResizeRight,
}

/// Returns `Some` if `pos` is inside `rect`, classifying whether the hit
/// is a body-grab or a right-edge resize-grab. Returns `None` otherwise.
pub fn hit_test_note(rect: Rectangle, pos: Point) -> Option<NoteEdge> {
    if pos.x < rect.x
        || pos.x > rect.x + rect.width
        || pos.y < rect.y
        || pos.y > rect.y + rect.height
    {
        return None;
    }
    if (rect.x + rect.width) - pos.x < RESIZE_EDGE_PX {
        Some(NoteEdge::ResizeRight)
    } else {
        Some(NoteEdge::Body)
    }
}

/// Canvas-local pixel rectangle for `note`, given the layout and
/// viewport. Shared by the draw routine and marquee hit testing so both
/// agree on where a note actually sits on screen.
pub fn note_rect(
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

/// Axis-aligned rectangle overlap test (touching edges don't count).
pub fn rects_intersect(a: Rectangle, b: Rectangle) -> bool {
    a.x < b.x + b.width
        && a.x + a.width > b.x
        && a.y < b.y + b.height
        && a.y + a.height > b.y
}

/// Normalised rectangle spanning the two corner points, so a marquee
/// dragged in any direction yields a positive-size rect.
pub fn rect_from_points(a: Point, b: Point) -> Rectangle {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rectangle {
        x,
        y,
        width: (a.x - b.x).abs(),
        height: (a.y - b.y).abs(),
    }
}

/// Indices of the notes whose on-screen rectangle intersects `marquee`
/// (all in canvas-local coordinates). Used by the rubber-band select.
pub fn notes_in_marquee(
    notes: &[resonance_audio::types::MidiNote],
    layout: &PianoRollLayout,
    viewport: &PianoRollViewport,
    marquee: Rectangle,
) -> Vec<usize> {
    notes
        .iter()
        .enumerate()
        .filter(|(_, n)| rects_intersect(note_rect(layout, viewport, n), marquee))
        .map(|(i, _)| i)
        .collect()
}

/// Styling for a single note rectangle.
pub struct NoteStyle {
    /// Stroke colour applied to the rounded-rect outline.
    pub stroke: Color,
    /// Stroke width in pixels.
    pub stroke_width: f32,
    /// Optional text label drawn inside large notes (skipped when the
    /// rectangle is too small to read). Use `None` to suppress.
    pub label: Option<String>,
}

impl NoteStyle {
    /// Default style: `theme::ACCENT_LINE` stroke at 1 px, no label.
    pub fn plain() -> Self {
        Self {
            stroke: theme::ACCENT_LINE,
            stroke_width: 1.0,
            label: None,
        }
    }

    /// Highlight a selected note with the brighter `theme::ACCENT`
    /// stroke at 1.5 px.
    pub fn selected() -> Self {
        Self {
            stroke: theme::ACCENT,
            stroke_width: 1.5,
            label: None,
        }
    }
}

/// Draw a single MIDI note rectangle. Velocity raises the fill alpha so
/// harder hits read denser without changing hue; the rectangle is
/// rounded at large sizes and stroked with the supplied accent colour.
/// `label`, if present and the rectangle exceeds the inline-text size
/// threshold, is drawn in BG_0 at the top-left.
pub fn draw_note(
    frame: &mut canvas::Frame,
    rect: Rectangle,
    velocity: f32,
    style: NoteStyle,
) {
    let v = velocity.clamp(0.0, 1.0);
    let fill = Color {
        a: 0.55 + 0.40 * v,
        ..theme::ACCENT_SOFT
    };
    let path = if rect.width >= 4.0 && rect.height >= 4.0 {
        canvas::Path::rounded_rectangle(
            Point::new(rect.x, rect.y),
            Size::new(rect.width, rect.height),
            2.0.into(),
        )
    } else {
        canvas::Path::rectangle(
            Point::new(rect.x, rect.y),
            Size::new(rect.width, rect.height),
        )
    };
    frame.fill(&path, fill);
    frame.stroke(
        &path,
        canvas::Stroke::default()
            .with_color(style.stroke)
            .with_width(style.stroke_width),
    );

    if let Some(label) = style.label {
        if rect.width > 28.0 && rect.height > 8.0 {
            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(rect.x + 4.0, rect.y + 1.0),
                color: Color {
                    a: 0.85,
                    ..theme::BG_0
                },
                size: (rect.height * 0.75).min(10.0).into(),
                font: theme::MONO_FONT,
                ..canvas::Text::default()
            });
        }
    }
}

/// Draw the piano keyboard column on the left of a canvas. White-key
/// bars use `theme::BG_3`, black-key bars `theme::BG_0`, with octave
/// labels (C0, C1, ...) at every C row when zoomed in enough. The
/// keyboard reads as its own card via a `theme::LINE_2` right-edge
/// hairline.
pub fn draw_keyboard(
    frame: &mut canvas::Frame,
    layout: &PianoRollLayout,
    viewport: &PianoRollViewport,
) {
    frame.fill_rectangle(
        Point::new(0.0, layout.grid_top),
        Size::new(layout.keyboard_w, layout.grid_h),
        theme::BG_2,
    );

    for midi_note in 0..NOTE_COUNT {
        let y = layout.grid_top + viewport.note_to_y_local(midi_note);
        let h = viewport.zoom_y;

        if y + h < layout.grid_top || y > layout.grid_top + layout.grid_h {
            continue;
        }

        let black = is_black_key(midi_note);
        let key_color = if black { theme::BG_0 } else { theme::BG_3 };
        let key_w = if black {
            layout.keyboard_w * 0.65
        } else {
            layout.keyboard_w - 1.0
        };

        frame.fill_rectangle(
            Point::new(0.0, y),
            Size::new(key_w, (h - 1.0).max(1.0)),
            key_color,
        );

        if midi_note % 12 == 0 && h >= 8.0 {
            frame.fill_text(canvas::Text {
                content: note_name(midi_note),
                position: Point::new(2.0, y + 1.0),
                color: theme::TEXT_3,
                size: (h * 0.7).min(10.0).into(),
                font: theme::MONO_FONT,
                ..canvas::Text::default()
            });
        }
    }
    frame.fill_rectangle(
        Point::new(layout.keyboard_w - 1.0, layout.grid_top),
        Size::new(1.0, layout.grid_h),
        theme::LINE_2,
    );
}
