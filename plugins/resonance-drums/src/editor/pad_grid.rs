//! The pad grid (§6.2): the kit's 30 pads as a 6×5 grid of cells.
//!
//! A cell shows the kit's name for the pad (`_meta.pieces`,
//! [`crate::pad_map`]) and its note; it **lights on each hit** (the
//! sampler's [`crate::last_hit`]), is outlined when selected, and is
//! dimmed when the kit has no recording for the pad (D7: silent).
//!
//! The cells grow with the window, keeping their shape: the grid takes
//! [`GRID_SHARE`] of the body, less whatever the inspector needs, and
//! the cells fill it as far as the height allows.
//!
//! Clicking a cell selects it and plays it. Where in the cell you click
//! is how hard: the bottom edge is the softest hit, the top the hardest —
//! hovering shows the velocity a click there plays (`v87`) over a faint
//! gradient.
//!
//! Mute is a switch in the inspector and in the Mix table (both bound to
//! the one `pad_N_mute`); the grid only *shows* it — a muted cell's name
//! is dimmed with a small `M`, and it never lights, since it plays
//! nothing. It is not a control here.
//!
//! Cheap per frame: the names are the kit's `Arc`'d strings, the note
//! labels are built once ([`note_labels`]), and the text goes through
//! egui's galley cache.

use std::sync::OnceLock;

use plugin_gui_core::egui;

use crate::drum_map::{NUM_PADS, PAD_MAPPINGS};

use super::app::{DrumsEditorApp, HIT_FLASH_SECS};
use super::{probe, theme};

/// Columns and rows of the grid.
pub(crate) const COLS: usize = 6;
pub(crate) const ROWS: usize = NUM_PADS.div_ceil(COLS);
/// Gap between cells.
const CELL_GAP: f32 = 6.0;
/// A cell's smallest size; below it the grid scrolls.
const CELL_MIN: egui::Vec2 = egui::vec2(46.0, 46.0);
/// A cell's width over its height: cells grow with the window in this
/// shape (the 92×76 cell the grid was designed at).
const CELL_ASPECT: f32 = 92.0 / 76.0;
/// The softest velocity a click plays (at the cell's bottom edge).
const MIN_CLICK_VELOCITY: f32 = 0.05;

/// The share of the body the grid takes when the inspector can spare it.
pub(crate) const GRID_SHARE: f32 = 0.58;
/// The width the inspector keeps, whatever the grid would like.
pub(crate) const INSPECTOR_MIN_W: f32 = 440.0;
/// The narrowest grid column: six of the smallest cells.
const GRID_MIN_W: f32 = COLS as f32 * CELL_MIN.x + (COLS - 1) as f32 * CELL_GAP + 2.0 * PAD_MARGIN;

/// The grid column's width for `avail` (grid + inspector, gaps excluded):
/// [`GRID_SHARE`] of it, less what the inspector needs
/// ([`INSPECTOR_MIN_W`]), but never under six of the smallest cells.
pub(crate) fn preferred_width(avail: f32) -> f32 {
    (avail * GRID_SHARE)
        .min(avail - INSPECTOR_MIN_W)
        .max(GRID_MIN_W)
        .min(avail)
        .max(0.0)
}

/// The cell size that fills `avail` (the grid's scroll area): as wide as
/// six fit, no taller than five fit, in the cells' own shape — and no
/// smaller than [`CELL_MIN`], where the grid scrolls instead.
fn cell_size(avail: egui::Vec2) -> egui::Vec2 {
    let w_fit = (avail.x - (COLS - 1) as f32 * CELL_GAP) / COLS as f32;
    let h_fit = (avail.y - (ROWS - 1) as f32 * CELL_GAP) / ROWS as f32;
    let w = w_fit.min(h_fit * CELL_ASPECT).max(CELL_MIN.x);
    egui::vec2(w, (w / CELL_ASPECT).max(CELL_MIN.y))
}

/// The card's inner margin.
const PAD_MARGIN: f32 = 12.0;

/// What a click on a cell asked for.
pub(crate) struct CellClick {
    pub pad: usize,
    /// The velocity to play the pad at (0..1), unless the kit has no
    /// recording for it.
    pub audition: Option<f32>,
}

/// Every pad's note label, `D1 · 38`, built once.
fn note_labels() -> &'static [String; NUM_PADS] {
    static LABELS: OnceLock<[String; NUM_PADS]> = OnceLock::new();
    LABELS.get_or_init(|| {
        std::array::from_fn(|i| {
            let note = PAD_MAPPINGS[i].note;
            format!("{} · {note}", note_name(note))
        })
    })
}

/// `C1` for MIDI 36 (MIDI 60 = C4).
pub(crate) fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", NAMES[note as usize % 12], note as i32 / 12 - 1)
}

/// The note label of `pad`.
pub(crate) fn note_label(pad: usize) -> &'static str {
    &note_labels()[pad]
}

/// The velocity a click at `y` plays in a cell spanning `rect`: the
/// bottom edge is the softest, the top the hardest.
pub(crate) fn click_velocity(rect: egui::Rect, y: f32) -> f32 {
    let t = 1.0 - (y - rect.top()) / rect.height().max(1.0);
    MIN_CLICK_VELOCITY + (1.0 - MIN_CLICK_VELOCITY) * t.clamp(0.0, 1.0)
}

/// Draw the grid card. Returns the cell the user clicked, if any.
pub(crate) fn draw(ui: &mut egui::Ui, app: &DrumsEditorApp, now: f64) -> Option<CellClick> {
    let mut click = None;
    let shown = super::controls::card().show(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 8.0);
        ui.horizontal(|ui| {
            super::controls::heading(ui, "PADS");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new("click to play · higher = harder")
                            .color(theme::TEXT_4)
                            .size(10.0),
                    )
                    .truncate(),
                );
            });
        });
        egui::ScrollArea::both()
            .id_salt("pad_grid_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let cell = cell_size(ui.available_size());
                let size = egui::vec2(
                    COLS as f32 * cell.x + (COLS - 1) as f32 * CELL_GAP,
                    ROWS as f32 * cell.y + (ROWS - 1) as f32 * CELL_GAP,
                );
                let (grid, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                let kit = app.bridge.kit_pads.current();
                for pad in 0..NUM_PADS {
                    let (col, row) = (pad % COLS, pad / COLS);
                    let min = grid.min
                        + egui::vec2(col as f32 * (cell.x + CELL_GAP), row as f32 * (cell.y + CELL_GAP));
                    let rect = egui::Rect::from_min_size(min, cell);
                    let kit_pad = &kit.pads[pad];
                    let flash = (1.0 - (now - app.hit_at[pad]) / HIT_FLASH_SECS).clamp(0.0, 1.0);
                    let cell_state = Cell {
                        name: &kit_pad.name,
                        present: kit_pad.present,
                        selected: app.selected_pad == pad,
                        muted: app.params.pads[pad].mute.value(),
                        flash: flash as f32,
                    };
                    if let Some(c) = draw_cell(ui, rect, pad, &cell_state) {
                        click = Some(c);
                    }
                }
            });
    });
    probe(ui, "pad_grid", shown.response.rect);
    click
}

/// What one cell shows.
struct Cell<'a> {
    name: &'a str,
    present: bool,
    selected: bool,
    muted: bool,
    /// 1 at a hit, falling to 0 over [`HIT_FLASH_SECS`].
    flash: f32,
}

fn draw_cell(ui: &mut egui::Ui, rect: egui::Rect, pad: usize, cell: &Cell<'_>) -> Option<CellClick> {
    let response = ui
        .interact(rect, ui.id().with(("pad_cell", pad)), egui::Sense::click())
        .on_hover_text(if !cell.present {
            "Not in this kit: this pad is silent"
        } else if cell.muted {
            "Muted — a click selects it but plays nothing. Unmute it in the \
             inspector or the Mix table."
        } else {
            "Click to select and play — higher in the cell plays harder"
        });
    probe(ui, format_args!("pad_cell.{pad}"), rect);
    if !cell.present {
        probe(ui, format_args!("pad_cell.{pad}.absent"), rect);
    }
    if !ui.is_rect_visible(rect) {
        return None;
    }
    let p = ui.painter_at(rect.expand(2.0));

    let base = if cell.present { theme::BG_1 } else { theme::BG_0 };
    let fill = if cell.flash > 0.0 {
        lerp_color(base, theme::ACCENT, 0.7 * cell.flash)
    } else if response.hovered() && cell.present {
        theme::BG_3
    } else {
        base
    };
    p.rect_filled(rect, 6.0, fill);
    let stroke = if cell.selected {
        egui::Stroke::new(1.5, theme::ACCENT_SOFT)
    } else {
        egui::Stroke::new(1.0, theme::LINE)
    };
    p.rect_stroke(rect, 6.0, stroke, egui::StrokeKind::Inside);

    // Velocity by height, made visible: on a playable cell under the
    // pointer, a faint gradient (harder at the top) and the velocity a
    // click right there would play.
    let hover_y = response
        .hover_pos()
        .filter(|_| cell.present && !cell.muted && cell.flash <= 0.0)
        .map(|pos| pos.y);
    if let Some(y) = hover_y {
        paint_velocity_gradient(&p, rect);
        let v = crate::last_hit::midi_velocity(click_velocity(rect, y));
        let at = egui::pos2(rect.right() - 7.0, y.clamp(rect.top() + 8.0, rect.bottom() - 8.0));
        let shown = p.text(
            at,
            egui::Align2::RIGHT_CENTER,
            format!("v{v}"),
            egui::FontId::monospace(9.5),
            theme::TEXT_1,
        );
        probe(ui, "pad_cell.velocity", shown);
    }

    let inner = rect.shrink2(egui::vec2(7.0, 6.0));
    let name_color = match (cell.present, cell.selected) {
        (false, _) => theme::TEXT_4,
        _ if cell.muted => theme::TEXT_3,
        (true, true) => theme::TEXT_1,
        (true, false) => theme::TEXT_2,
    };
    // The name: wrapped onto two lines at most, elided past that.
    let mut job = egui::text::LayoutJob::single_section(
        cell.name.to_string(),
        egui::TextFormat::simple(egui::FontId::proportional(11.0), name_color),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: inner.width() - if cell.muted { 10.0 } else { 0.0 },
        max_rows: 2,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    let galley = ui.painter().layout_job(job);
    p.galley(inner.left_top(), galley, name_color);

    p.text(
        inner.left_bottom(),
        egui::Align2::LEFT_BOTTOM,
        note_label(pad),
        egui::FontId::monospace(9.0),
        if cell.present { theme::TEXT_3 } else { theme::TEXT_4 },
    );
    if cell.muted {
        // An indicator, not a control: the mute switches are the
        // inspector's and the Mix table's.
        p.text(
            inner.right_top(),
            egui::Align2::RIGHT_TOP,
            "M",
            egui::FontId::proportional(9.0),
            theme::WARM,
        );
    }

    if response.clicked() {
        let y = response.interact_pointer_pos().map_or(rect.center().y, |p| p.y);
        return Some(CellClick {
            pad,
            audition: cell.present.then(|| click_velocity(rect, y)),
        });
    }
    None
}

/// A faint vertical wash over `rect`: accent at the top (hard), nothing
/// at the bottom (soft).
fn paint_velocity_gradient(p: &egui::Painter, rect: egui::Rect) {
    let top = theme::ACCENT.gamma_multiply(0.22);
    let bottom = egui::Color32::TRANSPARENT;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(egui::Shape::mesh(mesh));
}

fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}
