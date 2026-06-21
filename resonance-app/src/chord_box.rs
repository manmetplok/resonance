//! Backend-agnostic chord-box (fretboard chord-diagram) layout.
//!
//! The drawing *convention* for a single chord diagram — where the nut,
//! fret lines, string lines, open/mute markers, finger dots and labels go
//! given a voicing — used to live only inside the PDF exporter
//! (`chord_sheet_pdf::draw_fretboard_cell`). This module factors that
//! convention out into a pure, backend-agnostic [`layout`] function that
//! computes element positions for one cell. Each backend then renders the
//! returned [`ChordBoxLayout`]:
//!
//! - the PDF exporter emits `printpdf` ops (output byte-for-byte unchanged
//!   from the inlined code);
//! - the Performance view (todo E) emits Iced `Canvas` primitives.
//!
//! The caller passes the cell's top-left `origin` and a [`Dims`] describing
//! the cell geometry in its own units (mm for PDF, px for Canvas), so the
//! *placement rules* are shared while the absolute scale stays
//! backend-specific. Passing `origin = (0.0, 0.0)` yields **cell-relative**
//! coordinates (the recommended frame for the Canvas backend); the PDF
//! backend passes the cell's page position so the arithmetic — and hence
//! the emitted coordinate bytes — matches the original inlined code exactly.
//! Colours and exact text metrics (centring nudges, fonts) stay in each
//! backend: this module only decides *what* goes *where* and which dot is
//! the root.

use resonance_music_theory::{Chord, PitchClass, Tuning};

/// Cell geometry, in the caller's own unit (mm for PDF, px for Canvas).
///
/// Mirrors the constants the PDF exporter used inline; passing them in keeps
/// the placement logic shared while letting each backend pick its scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dims {
    /// Full width of the chord cell.
    pub cell_w: f32,
    /// Vertical space reserved for the chord name above the header row.
    pub chord_name_h: f32,
    /// Vertical space between the chord name and the nut (holds string
    /// labels and the open/mute marker row).
    pub header_h: f32,
    /// Spacing between adjacent fret lines.
    pub fret_spacing: f32,
    /// Number of fret cells drawn (board shows frets `0..=fret_count`).
    pub fret_count: u8,
    /// Horizontal spacing between adjacent strings.
    pub string_spacing: f32,
    /// Radius of a finger dot.
    pub dot_r: f32,
}

/// Marker drawn above an un-fretted string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    /// Played open (fret 0) — conventionally an `O`.
    Open,
    /// Muted / not played — conventionally an `X`.
    Mute,
}

/// How the top edge of the board is anchored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nut {
    /// Diagram is nut-anchored: draw the heavy nut bar at the top.
    Open,
    /// Diagram is a moveable box starting at this (1-based) fret: draw a
    /// `"{n}f"` position label instead of a nut bar.
    StartFret(u8),
}

/// One string of the diagram, left (lowest) to right (highest).
#[derive(Debug, Clone, PartialEq)]
pub struct StringInfo {
    /// Cell-relative x of the string line.
    pub x: f32,
    /// Tuning label for the string (e.g. `"E"`, `"B"`).
    pub label: &'static str,
    /// Open/mute marker above the board, if the string is not fretted.
    pub marker: Option<Marker>,
}

/// A fretted finger position.
#[derive(Debug, Clone, PartialEq)]
pub struct Dot {
    /// Cell-relative centre x.
    pub x: f32,
    /// Cell-relative centre y.
    pub y: f32,
    /// Dot radius (copied from [`Dims::dot_r`] for convenience).
    pub r: f32,
    /// Name of the sounding note (e.g. `"C#"`).
    pub note: &'static str,
    /// Whether this dot is the chord's root/bass — drawn as the accent.
    pub is_root: bool,
}

/// Fully-resolved geometry for one chord diagram. Coordinates are in the
/// frame chosen by the `origin` passed to [`layout`] (cell-relative when
/// `origin = (0.0, 0.0)`).
#[derive(Debug, Clone, PartialEq)]
pub struct ChordBoxLayout {
    /// Rendered chord name (e.g. `"C"`, `"Dm7/A"`).
    pub name: String,
    /// x of the cell centre (anchor for centring the name).
    pub cell_center_x: f32,
    /// y of the top of the chord name.
    pub name_y: f32,
    /// y of the header row (string labels / markers).
    pub header_y: f32,
    /// x of the left edge of the board.
    pub board_x: f32,
    /// Width of the board (left-most to right-most string).
    pub board_w: f32,
    /// y of the nut / first fret line.
    pub nut_y: f32,
    /// Height of the fretted area (`fret_count * fret_spacing`).
    pub board_h: f32,
    /// Whether to draw a nut bar or a start-fret label.
    pub nut: Nut,
    /// Strings, low to high.
    pub strings: Vec<StringInfo>,
    /// y of each fret line, `0..=fret_count`.
    pub fret_ys: Vec<f32>,
    /// Fretted finger dots.
    pub dots: Vec<Dot>,
}

/// Compute the layout for one chord diagram.
///
/// `origin` is the cell's top-left corner in the caller's frame; pass
/// `(0.0, 0.0)` for cell-relative coordinates. `frets[i]` is the fret played
/// on string `i` (`Some(0)` = open, `None` = muted), as produced by
/// `resonance_music_theory::fretboard_voicing`. `start_fret` is the voicing's
/// window start (`0` = nut-anchored). The root/bass note of `chord` is
/// highlighted as the accent dot.
///
/// The expression order mirrors the original PDF exporter so a PDF backend
/// passing the page-space cell origin reproduces its output byte-for-byte.
pub fn layout(
    dims: &Dims,
    origin: (f32, f32),
    tuning: &Tuning,
    frets: &[Option<u8>],
    start_fret: u8,
    chord: &Chord,
) -> ChordBoxLayout {
    let (origin_x, origin_y) = origin;
    let n = tuning.string_count();
    let board_w = (n.saturating_sub(1)) as f32 * dims.string_spacing;
    let board_x = origin_x + (dims.cell_w - board_w) / 2.0;
    let name_y = origin_y;
    let header_y = name_y + dims.chord_name_h;
    let nut_y = header_y + dims.header_h;
    let board_h = dims.fret_count as f32 * dims.fret_spacing;
    let root_pc = chord.bass.unwrap_or(chord.root).to_semitone();

    // Strings (labels + open/mute markers), low on the left.
    let strings: Vec<StringInfo> = (0..n)
        .map(|i| {
            let x = board_x + i as f32 * dims.string_spacing;
            let label = tuning.labels.get(i).copied().unwrap_or("");
            let marker = match frets.get(i).copied().flatten() {
                Some(0) => Some(Marker::Open),
                None => Some(Marker::Mute),
                Some(_) => None,
            };
            StringInfo { x, label, marker }
        })
        .collect();

    // Fret lines, nut (0) through `fret_count`.
    let fret_ys: Vec<f32> = (0..=dims.fret_count)
        .map(|f| nut_y + f as f32 * dims.fret_spacing)
        .collect();

    let nut = if start_fret == 0 {
        Nut::Open
    } else {
        Nut::StartFret(start_fret)
    };

    // Finger dots — one per fretted string within the visible window.
    let dots: Vec<Dot> = frets
        .iter()
        .enumerate()
        .filter_map(|(i, fret_opt)| {
            let fret = (*fret_opt)?;
            if fret == 0 {
                return None;
            }
            // Map the absolute fret onto the visible window: nut-anchored
            // diagrams show the fret directly, boxed ones count from the
            // start fret.
            let display_fret = if start_fret == 0 {
                fret
            } else {
                fret - start_fret + 1
            };
            if display_fret > dims.fret_count {
                return None;
            }
            let x = board_x + i as f32 * dims.string_spacing;
            let y = nut_y + (display_fret as f32 - 0.5) * dims.fret_spacing;
            let note_pc = (tuning.open[i] + fret) % 12;
            Some(Dot {
                x,
                y,
                r: dims.dot_r,
                note: PitchClass::from_semitone(note_pc).as_str(),
                is_root: note_pc == root_pc,
            })
        })
        .collect();

    ChordBoxLayout {
        name: chord.to_string(),
        cell_center_x: origin_x + dims.cell_w / 2.0,
        name_y,
        header_y,
        board_x,
        board_w,
        nut_y,
        board_h,
        nut,
        strings,
        fret_ys,
        dots,
    }
}
