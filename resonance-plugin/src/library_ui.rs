//! egui skin over [`crate::library_view::BrowserModel`]: the virtualised
//! library list with ☆/★ toggles and keyboard navigation, the search
//! field, facet menus, the confirm-in-place delete row and the tag editor
//! row.
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
    /// Widths of [`LibraryRows::column`]s, in order; extra columns are not
    /// drawn.
    pub columns: &'a [ColumnSpec],
    /// The key of the item currently loaded: drawn with an accent border.
    pub loaded: Option<&'a str>,
    /// Draw the ☆/★ toggle at the left of each row.
    pub show_star: bool,
    /// Draw [`LibraryRows::subtitle`] as a second, dimmer line (give the
    /// rows room: 34 px or more).
    pub show_subtitle: bool,
    /// ↑/↓ move the selection, Enter activates it, Esc reports
    /// [`ListResponse::escaped`] — while no text field is being edited,
    /// nor was when the frame began ([`text_field_had_focus`]). Enter only
    /// activates a selection that is in the current view.
    pub keyboard: bool,
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
            show_subtitle: false,
            keyboard: true,
            is_error: None,
        }
    }
}

/// What the user did in the list this frame (row indices).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListResponse {
    /// A row was clicked (it is now the model's selection).
    pub clicked: Option<usize>,
    /// A row was double-clicked, or Enter pressed on the selection (load
    /// it).
    pub double_clicked: Option<usize>,
    /// A row's star was clicked (toggle its favourite). Does not select.
    pub star_clicked: Option<usize>,
    /// ↑/↓ moved the selection to this row (audition it, if the browser
    /// auditions).
    pub moved: Option<usize>,
    /// Esc was pressed with the list focused and no text field active.
    pub escaped: bool,
}

/// Records, at the end of every pass, whether a text field had keyboard
/// focus — so the next pass can know it had focus *at its start*.
///
/// egui takes the focus away from a single-line `TextEdit` before any
/// caller code sees the key: Esc clears it in `Memory::begin_pass`, and
/// Enter surrenders it inside the field's own `show`. By the time a list
/// or an overlay asks [`egui::Context::text_edit_focused`], it is `false`
/// in exactly the frame where the key was meant for the field — so Esc
/// closed the overlay and Enter in the search box loaded the selected row.
struct TextFocusPlugin;

#[derive(Clone, Copy, Default)]
struct TextFocusAtEnd(bool);

fn text_focus_id() -> egui::Id {
    egui::Id::new("resonance_library_ui_text_focus_at_end")
}

impl egui::Plugin for TextFocusPlugin {
    fn debug_name(&self) -> &'static str {
        "resonance_library_ui_text_focus"
    }

    fn on_end_pass(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let focused = ctx.text_edit_focused();
        ctx.data_mut(|d| d.insert_temp(text_focus_id(), TextFocusAtEnd(focused)));
    }
}

/// Whether a text field is being typed in: focused now, or focused when
/// this frame began (so an Esc or Enter this frame was the field's — it
/// only left the field). Keyboard shortcuts that share Esc / Enter / the
/// arrows with a text field must check this, not
/// [`egui::Context::text_edit_focused`].
///
/// The first call on a context installs the end-of-pass hook that makes
/// it work, so on that one frame this only knows about the current focus.
pub fn text_field_had_focus(ctx: &egui::Context) -> bool {
    ctx.add_plugin(TextFocusPlugin);
    ctx.text_edit_focused()
        || ctx
            .data(|d| d.get_temp::<TextFocusAtEnd>(text_focus_id()))
            .is_some_and(|f| f.0)
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
    let row_h = opts.row_height;
    let salt = egui::Id::new(id_salt);

    // Keyboard, before layout, so the scroll can follow the move.
    let mut scroll_to: Option<usize> = None;
    if opts.keyboard && !text_field_had_focus(ui.ctx()) {
        let (up, down, enter, esc) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        let delta = i32::from(down) - i32::from(up);
        if delta != 0 {
            if let Some(row) = model.move_selection(rows, delta) {
                out.moved = Some(row);
                scroll_to = model.view().iter().position(|&r| r == row);
            }
        }
        // Only a selection the user can see: a row filtered out of the
        // view stays selected (clearing the search brings it back), but
        // Enter must not load what is not on screen.
        if enter {
            out.double_clicked = model
                .selected()
                .filter(|k| model.position_in_view(k).is_some())
                .and_then(|_| model.selected_row());
        }
        out.escaped = esc;
    }

    let view: Vec<usize> = model.view().to_vec();
    let mut area = egui::ScrollArea::vertical()
        .id_salt(salt)
        .auto_shrink([false, false]);
    if let Some(pos) = scroll_to {
        let visible = ui.available_height().max(row_h);
        let spacing = ui.spacing().item_spacing.y;
        let top = pos as f32 * (row_h + spacing);
        area = area.vertical_scroll_offset((top - visible * 0.5).max(0.0));
    }
    area.show_rows(ui, row_h, view.len(), |ui, range| {
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
    let two_lines = opts.show_subtitle && opts.row_height >= 30.0;
    let line_y = if two_lines {
        rect.min.y + opts.row_height * 0.36
    } else {
        rect.center().y
    };

    let star_rect = egui::Rect::from_min_size(
        egui::pos2(rect.min.x + 4.0, line_y - 8.0),
        egui::vec2(16.0, 16.0),
    );
    if resp.clicked() {
        out.clicked = Some(row);
    }
    if resp.double_clicked() {
        out.double_clicked = Some(row);
    }
    let mut title_elided = false;

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

        // Columns from the right edge inwards.
        let mut right = rect.max.x - 6.0;
        let mut col_rects = Vec::with_capacity(opts.columns.len());
        let count = (0..opts.columns.len())
            .take_while(|&c| rows.column(row, c).is_some())
            .count();
        for (c, spec) in opts.columns.iter().enumerate().take(count).rev() {
            let left = right - spec.width;
            col_rects.push((c, egui::Rect::from_x_y_ranges(left..=right, rect.y_range()), *spec));
            right = left - 8.0;
        }
        for (c, col_rect, spec) in col_rects {
            let Some(text) = rows.column(row, c) else {
                continue;
            };
            let p = painter.with_clip_rect(col_rect);
            let (x, align) = if spec.align_right {
                (col_rect.max.x, egui::Align2::RIGHT_CENTER)
            } else {
                (col_rect.min.x, egui::Align2::LEFT_CENTER)
            };
            p.text(egui::pos2(x, line_y), align, text, small.clone(), theme::TEXT_2);
        }
        let title_left = if opts.show_star {
            star_rect.max.x + 6.0
        } else {
            rect.min.x + 6.0
        };
        // Elided with "…" rather than cut off at the first column: a cut
        // title reads as a different, shorter name. The full title is on
        // the row's hover.
        let title_w = (right - title_left).max(0.0);
        let galley = elided(&painter, rows.title(row), font, title_color, title_w);
        title_elided = galley.elided;
        let pos = egui::pos2(title_left, line_y - galley.size().y * 0.5);
        painter.galley(pos, galley, title_color);
        if two_lines {
            let sub_rect = egui::Rect::from_x_y_ranges(title_left..=rect.max.x - 6.0, rect.y_range());
            let subtitle = rows.subtitle(row);
            if !subtitle.is_empty() {
                painter.with_clip_rect(sub_rect).text(
                    egui::pos2(title_left, rect.min.y + opts.row_height * 0.74),
                    egui::Align2::LEFT_CENTER,
                    subtitle,
                    small,
                    if error { theme::BAD } else { theme::TEXT_3 },
                );
            }
        }
    }
    if title_elided {
        resp.on_hover_text(rows.title(row));
    }
    out
}

/// `text` laid out on one line, elided with "…" to `width`.
fn elided(
    painter: &egui::Painter,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    painter.layout_job(job)
}

/// A facet filter as a menu button: `label` (with the selection) listing
/// every value with its count; clicking a value toggles it and the menu
/// stays open for the next one. Returns whether the selection changed.
pub fn facet_menu(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    label: &str,
    model: &mut BrowserModel,
    rows: &dyn LibraryRows,
    facet: &str,
) -> bool {
    let selected = model.facet_selection(facet);
    // The combo draws its own ▾.
    let text = match selected.len() {
        0 => label.to_string(),
        1 => format!("{label}: {}", selected[0]),
        n => format!("{label}: {n}"),
    };
    let mut changed = false;
    egui::ComboBox::from_id_salt(egui::Id::new(id_salt))
        .selected_text(text)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
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
    /// The second click: delete this key (the one that was armed — delete
    /// this, not whatever is displayed).
    Confirmed(String),
    Cancelled,
}

/// The confirm-in-place line for `key`: `prompt [Delete] [Cancel]`, with
/// an optional dimmer `detail` line under it ("Used by 2 open amps …").
/// Draws nothing unless the model's pending delete is exactly `key`, so a
/// delete armed on one row can never be confirmed on another.
pub fn confirm_delete_row(
    ui: &mut egui::Ui,
    model: &mut BrowserModel,
    key: &str,
    prompt: &str,
    detail: Option<&str>,
) -> ConfirmOutcome {
    confirm_delete_row_blocked(ui, model, key, prompt, detail, None)
}

/// Width the `[Delete] [Cancel]` pair needs beside the prompt.
const CONFIRM_BUTTONS_W: f32 = 130.0;

/// [`confirm_delete_row`] with `Delete` disabled while `blocked` says why
/// (e.g. "wait for the scan to finish"). The delete stays armed, so the
/// user confirms once the reason has passed.
///
/// The prompt is one line, elided with "…" (full text on hover). When it
/// does not fit beside the buttons they go on their own line under it, so
/// a long name can never push them past the panel's edge.
pub fn confirm_delete_row_blocked(
    ui: &mut egui::Ui,
    model: &mut BrowserModel,
    key: &str,
    prompt: &str,
    detail: Option<&str>,
    blocked: Option<&str>,
) -> ConfirmOutcome {
    if model.pending_delete() != Some(key) {
        return ConfirmOutcome::None;
    }
    let mut outcome = ConfirmOutcome::None;
    let mut buttons = |ui: &mut egui::Ui, model: &mut BrowserModel| {
        let delete = egui::Button::new(egui::RichText::new("Delete").color(theme::BG_0))
            .fill(theme::BAD);
        let r = ui.add_enabled(blocked.is_none(), delete);
        let r = match blocked {
            Some(why) => r.on_disabled_hover_text(why),
            None => r,
        };
        if r.clicked() {
            if let Some(key) = model.confirm_delete() {
                outcome = ConfirmOutcome::Confirmed(key);
            }
        }
        if ui.button("Cancel").clicked() {
            model.cancel_delete();
            outcome = ConfirmOutcome::Cancelled;
        }
        if let Some(why) = blocked {
            ui.add(
                egui::Label::new(egui::RichText::new(why).size(11.0).color(theme::TEXT_3))
                    .truncate(),
            );
        }
    };
    ui.vertical(|ui| {
        let text = egui::RichText::new(prompt).color(theme::BAD);
        let natural = egui::WidgetText::from(text.clone())
            .into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Body,
            )
            .size()
            .x;
        if natural + CONFIRM_BUTTONS_W <= ui.available_width() {
            ui.horizontal(|ui| {
                ui.label(text);
                buttons(ui, model);
            });
        } else {
            ui.add(egui::Label::new(text).truncate())
                .on_hover_text(prompt);
            ui.horizontal(|ui| buttons(ui, model));
        }
        if let Some(detail) = detail {
            ui.add(
                egui::Label::new(egui::RichText::new(detail).size(11.0).color(theme::TEXT_3))
                    .wrap(),
            );
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
/// `draft` is the caller-owned text buffer; `suggestions` (typically
/// `MarksStore::complete_tag`) are offered while the draft is non-empty —
/// not only while the field has focus, because clicking a suggestion takes
/// the focus away on that very frame.
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
        if !draft.trim().is_empty() {
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
