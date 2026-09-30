//! egui skin over [`crate::library_view::BrowserModel`]: the virtualised
//! library list with ☆/★ toggles, the search field, facet menus, the
//! confirm-in-place delete row and the tag editor row.
//!
//! Every function here is a thin translation of clicks into model calls;
//! the behaviour is in `library_view` and tested there without a window.
//! Shared by the NAM model manager and the plugin preset browser.

use plugin_gui_core::egui;
use plugin_gui_core::theme::lavender as theme;
use plugin_gui_core::widgets::{star_toggle_sized, tag_pill};

use crate::library_view::{BrowserModel, LibraryRows};

/// One display column after the title.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnSpec {
    /// Width in px.
    pub width: f32,
    /// Right-align the text (sizes, counts).
    pub align_right: bool,
}

impl ColumnSpec {
    pub const fn left(width: f32) -> Self {
        Self {
            width,
            align_right: false,
        }
    }

    pub const fn right(width: f32) -> Self {
        Self {
            width,
            align_right: true,
        }
    }
}

/// How [`library_list`] draws.
#[derive(Clone, Copy)]
pub struct ListOptions<'a> {
    /// Height of every row, px (fixed, so the list can virtualise).
    pub row_height: f32,
    /// Widths of [`LibraryRows::columns`], in order; extra columns are not
    /// drawn.
    pub columns: &'a [ColumnSpec],
    /// The key of the item currently loaded: drawn with an accent border.
    pub loaded: Option<&'a str>,
    /// Draw the ☆/★ toggle at the left of each row.
    pub show_star: bool,
    /// Rows to draw in the danger colour (e.g. unreadable files).
    pub is_error: Option<&'a dyn Fn(usize) -> bool>,
}

impl Default for ListOptions<'_> {
    fn default() -> Self {
        Self {
            row_height: 24.0,
            columns: &[],
            loaded: None,
            show_star: true,
            is_error: None,
        }
    }
}

/// What the user did in the list this frame (row indices).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListResponse {
    /// A row was clicked (it is now the model's selection).
    pub clicked: Option<usize>,
    /// A row was double-clicked (load it).
    pub double_clicked: Option<usize>,
    /// A row's star was clicked (toggle its favourite). Does not select.
    pub star_clicked: Option<usize>,
}

/// The search field, bound to the model's query. Returns the text edit's
/// response (`changed()` when the query moved).
pub fn search_field(ui: &mut egui::Ui, model: &mut BrowserModel, hint: &str, width: f32) -> egui::Response {
    let mut buf = model.query().to_string();
    let resp = ui.add(
        egui::TextEdit::singleline(&mut buf)
            .hint_text(hint)
            .desired_width(width),
    );
    if resp.changed() {
        model.set_query(buf);
    }
    resp
}

/// The virtualised list of the model's current view. Call
/// [`BrowserModel::refresh`] before it. Only the visible rows are laid out,
/// so a thousand rows cost one screen.
pub fn library_list(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    model: &mut BrowserModel,
    rows: &dyn LibraryRows,
    opts: &ListOptions<'_>,
) -> ListResponse {
    let mut out = ListResponse::default();
    let view: Vec<usize> = model.view().to_vec();
    let row_h = opts.row_height;
    let salt = egui::Id::new(id_salt);
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .auto_shrink([false, false])
        .show_rows(ui, row_h, view.len(), |ui, range| {
            for &row in &view[range] {
                let r = draw_row(ui, salt, model, rows, row, opts);
                if r.star_clicked.is_some() {
                    out.star_clicked = r.star_clicked;
                } else {
                    if r.clicked.is_some() {
                        out.clicked = r.clicked;
                    }
                    if r.double_clicked.is_some() {
                        out.double_clicked = r.double_clicked;
                    }
                }
            }
        });
    if let Some(row) = out.clicked.or(out.double_clicked) {
        model.select(rows.key(row).to_string());
    }
    out
}

fn draw_row(
    ui: &mut egui::Ui,
    salt: egui::Id,
    model: &BrowserModel,
    rows: &dyn LibraryRows,
    row: usize,
    opts: &ListOptions<'_>,
) -> ListResponse {
    let key = rows.key(row);
    let width = ui.available_width();
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(width, opts.row_height), egui::Sense::click());
    let selected = model.selected() == Some(key);
    let loaded = opts.loaded == Some(key);
    let mut out = ListResponse::default();

    let star_rect = egui::Rect::from_min_size(
        rect.min + egui::vec2(4.0, (opts.row_height - 16.0) * 0.5),
        egui::vec2(16.0, 16.0),
    );
    if resp.clicked() {
        out.clicked = Some(row);
    }
    if resp.double_clicked() {
        out.double_clicked = Some(row);
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        let fill = if selected {
            theme::BG_3
        } else if resp.hovered() {
            theme::BG_2
        } else {
            egui::Color32::TRANSPARENT
        };
        painter.rect_filled(rect.shrink2(egui::vec2(0.0, 1.0)), 4.0, fill);
        if loaded {
            painter.rect_stroke(
                rect.shrink2(egui::vec2(0.5, 1.5)),
                4.0,
                egui::Stroke::new(1.0, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
        }
    }
    // The star after the row background (so it is drawn on top) and after
    // the row's own interaction (so it wins the hit test).
    if opts.show_star {
        let fav = rows.marks(row).is_some_and(|m| m.favorite);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(star_rect)
                .id_salt(salt.with(("star", key))),
        );
        if star_toggle_sized(&mut child, fav, 16.0).clicked() {
            out.star_clicked = Some(row);
        }
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        let error = opts.is_error.is_some_and(|f| f(row));
        let title_color = if error { theme::BAD } else { theme::TEXT_1 };
        let font = egui::FontId::proportional(12.0);
        let small = egui::FontId::proportional(11.0);
        let y = rect.center().y;

        // Columns from the right edge inwards.
        let cols = rows.columns(row);
        let mut right = rect.max.x - 6.0;
        let mut col_rects = Vec::new();
        for spec in opts.columns.iter().take(cols.len()).rev() {
            let left = right - spec.width;
            col_rects.push((egui::Rect::from_x_y_ranges(left..=right, rect.y_range()), *spec));
            right = left - 8.0;
        }
        col_rects.reverse();
        for ((col_rect, spec), text) in col_rects.iter().zip(cols.iter()) {
            let p = painter.with_clip_rect(*col_rect);
            if spec.align_right {
                p.text(
                    egui::pos2(col_rect.max.x, y),
                    egui::Align2::RIGHT_CENTER,
                    text,
                    small.clone(),
                    theme::TEXT_2,
                );
            } else {
                p.text(
                    egui::pos2(col_rect.min.x, y),
                    egui::Align2::LEFT_CENTER,
                    text,
                    small.clone(),
                    theme::TEXT_2,
                );
            }
        }
        let title_left = if opts.show_star {
            star_rect.max.x + 6.0
        } else {
            rect.min.x + 6.0
        };
        let title_rect = egui::Rect::from_x_y_ranges(title_left..=right, rect.y_range());
        painter.with_clip_rect(title_rect).text(
            egui::pos2(title_left, y),
            egui::Align2::LEFT_CENTER,
            rows.title(row),
            font,
            title_color,
        );
    }
    out
}

/// A facet filter as a menu button: `label ▾` listing every value with its
/// count; clicking a value toggles it. Returns whether the selection
/// changed.
pub fn facet_menu(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    label: &str,
    model: &mut BrowserModel,
    rows: &dyn LibraryRows,
    facet: &str,
) -> bool {
    let selected = model.facet_selection(facet);
    let text = match selected.len() {
        0 => format!("{label} ▾"),
        1 => format!("{label}: {} ▾", selected[0]),
        n => format!("{label}: {n} ▾"),
    };
    let mut changed = false;
    egui::ComboBox::from_id_salt(egui::Id::new(id_salt))
        .selected_text(text)
        .show_ui(ui, |ui| {
            let counts = model.facet_counts(rows, facet);
            if counts.is_empty() {
                ui.label(egui::RichText::new("(none)").color(theme::TEXT_3));
            }
            if !model.facet_selection(facet).is_empty() && ui.selectable_label(false, "Any").clicked() {
                model.set_facet(facet, None);
                changed = true;
            }
            for c in counts {
                let line = format!("{}  {}", c.value, c.count);
                if ui.selectable_label(c.selected, line).clicked() {
                    model.toggle_facet(facet, &c.value);
                    changed = true;
                }
            }
        });
    changed
}

/// What a [`confirm_delete_row`] click did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmOutcome {
    None,
    /// The second click: delete this key.
    Confirmed(String),
    Cancelled,
}

/// The confirm-in-place line for the model's pending delete:
/// `prompt [Delete] [Cancel]`, with an optional dimmer `detail` line under
/// it ("Used by 2 open amps …"). Draws nothing when no delete is pending.
pub fn confirm_delete_row(
    ui: &mut egui::Ui,
    model: &mut BrowserModel,
    prompt: &str,
    detail: Option<&str>,
) -> ConfirmOutcome {
    if model.pending_delete().is_none() {
        return ConfirmOutcome::None;
    }
    let mut outcome = ConfirmOutcome::None;
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(prompt).color(theme::BAD));
            let delete = egui::Button::new(egui::RichText::new("Delete").color(theme::BG_0))
                .fill(theme::BAD);
            if ui.add(delete).clicked() {
                if let Some(key) = model.confirm_delete() {
                    outcome = ConfirmOutcome::Confirmed(key);
                }
            }
            if ui.button("Cancel").clicked() {
                model.cancel_delete();
                outcome = ConfirmOutcome::Cancelled;
            }
        });
        if let Some(detail) = detail {
            ui.label(egui::RichText::new(detail).size(11.0).color(theme::TEXT_3));
        }
    });
    outcome
}

/// What a [`tag_row`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagRowResponse {
    /// A tag to add (typed and submitted, or a completion picked).
    pub added: Option<String>,
    /// A tag whose × was clicked.
    pub removed: Option<String>,
}

/// Removable tag pills plus an inline `+ tag` field with completions.
/// `draft` is the caller-owned text buffer; `suggestions` are offered
/// under it while it has focus (typically `MarksStore::complete_tag`).
pub fn tag_row(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    tags: &[String],
    draft: &mut String,
    suggestions: &[String],
) -> TagRowResponse {
    let mut out = TagRowResponse::default();
    let id = egui::Id::new(id_salt);
    ui.horizontal_wrapped(|ui| {
        for t in tags {
            if tag_pill(ui, t, false, true).removed {
                out.removed = Some(t.clone());
            }
        }
        let resp = ui.add(
            egui::TextEdit::singleline(draft)
                .id(id.with("draft"))
                .hint_text("+ tag")
                .desired_width(90.0),
        );
        let submitted = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if submitted && !draft.trim().is_empty() {
            out.added = Some(std::mem::take(draft));
        }
        if resp.has_focus() && !suggestions.is_empty() {
            for s in suggestions.iter().take(6) {
                if tag_pill(ui, s, true, false).clicked {
                    out.added = Some(s.clone());
                    draft.clear();
                }
            }
        }
    });
    out
}
