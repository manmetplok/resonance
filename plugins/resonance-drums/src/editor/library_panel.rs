//! The Library overlay (drums-plugin-rework.md §6.5): a modal panel over a
//! dimmed editor with two tabs, `Installed | plok.org`.
//!
//! ```text
//! ┌ KIT LIBRARY  [ Installed | plok.org ]                       2 kits · 8.5 GB   [Close] ┐
//! │ [search name, tag, mic…______]  (★ only) (Recent)  Source ▾  Mics ▾  Sort ▾          │
//! │ ★ Drummica          35 pieces  14 setups  7 layers  3 RR   plok    8.5 GB   ← loaded │
//! │ ☆ IT Techno         19 pieces   1 setup   1 layer   1 RR   local   5.0 MB            │
//! │ ───────────────────────────────────────────────────────────────────────────────────── │
//! │ Drummica · plok.org · added 2026-04-12 · 2,835 samples · …/drumkits/Drummica          │
//! │ pieces: Kick, Kick (ohne Teppich), Snare, … (35)   articulations: —                   │
//! │ mics: Shure Beta 91 · Kick In, AKG D112 · Kick Out, …                                 │
//! │ tags: (rock ×) (metal ×) [+ tag]                     used in 1 open drum instance      │
//! │ [Load]  [Reveal]  [Re-download]  [Delete…]                                             │
//! │ [Import kit folder / .zip… ▾]  [Rescan]                        status / last error    │
//! └───────────────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! It is an `egui::Modal`: the backdrop is painted on the modal's own
//! foreground layer, beneath the panel — never above it, which is what made
//! the old Download Kits overlay a near-black screen (§1.2) — and it senses
//! clicks, so nothing reaches the pads behind it. Esc and Close close it; a
//! click on the backdrop does not. Esc typed in a text field only leaves
//! the field, and Esc with a delete armed only disarms it.
//!
//! The footer and the selected kit's action row (with the delete confirm)
//! are pinned at the bottom; the list and the detail facts share what is
//! left. A 35-piece kit's facts run long, and with the actions inside the
//! detail's scroll they sat below the fold at the minimum window size.
//!
//! The behaviour lives in the shared `BrowserModel` and `kit_rows`; this
//! file only turns clicks into their calls. Anything that hashes, copies
//! or deletes is a job (`jobs.rs`).

use std::path::PathBuf;

use plugin_gui_core::egui;
use plugin_gui_core::widgets;
use resonance_common::drumkit_library::{
    format_bytes, Entry, EntryStatus, ImportJob, LibraryError, Source,
};
use resonance_plugin::kit_rows::{sort_options, FACETS};
use resonance_plugin::library_ui::{self, ColumnSpec, ConfirmOutcome, ListOptions};
use resonance_plugin::library_view::Sort;

use super::app::{DrumsEditorApp, Queued};
use super::jobs::{JobDone, JobKind, PickKind};
use super::kit_browser::LoadKind;
use super::{plok_panel, probe, theme};
use crate::download::{Command, ServerKit};

/// Which tab is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Tab {
    #[default]
    Installed,
    Plok,
}

impl Tab {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Tab::Installed => "Installed",
            Tab::Plok => "plok.org",
        }
    }
}

/// The overlay's editor-only state. Not persisted.
#[derive(Default)]
pub(crate) struct LibraryPanelState {
    pub(crate) open: bool,
    pub(crate) tab: Tab,
}

/// The Installed tab's columns: pieces, mic setups, layers, RR, source,
/// size (`kit_rows::COLUMNS`).
const COLUMNS: [ColumnSpec; 6] = [
    ColumnSpec::right(62.0),
    ColumnSpec::right(64.0),
    ColumnSpec::right(56.0),
    ColumnSpec::right(40.0),
    ColumnSpec::left(52.0),
    ColumnSpec::right(60.0),
];

/// Space between the window edge and the panel.
const MARGIN: f32 = 20.0;
/// The panel frame's inner margin.
const FRAME_MARGIN: i8 = 12;

/// Open the overlay: on Installed, or on plok.org when nothing is
/// installed. A delete armed before is never still armed, and a notice
/// from the last visit is not shown again.
pub(crate) fn open(app: &mut DrumsEditorApp) {
    app.library_panel.open = true;
    app.browser.cancel_delete();
    app.browser.clear_notice();
    let empty = app.library.read().is_empty();
    set_tab(app, if empty { Tab::Plok } else { Tab::Installed });
}

/// Close the overlay, disarming any half-confirmed delete.
pub(crate) fn close(app: &mut DrumsEditorApp) {
    app.library_panel.open = false;
    app.browser.cancel_delete();
}

/// Show `tab`; opening plok.org fetches the index unless it is fresh.
pub(crate) fn set_tab(app: &mut DrumsEditorApp, tab: Tab) {
    app.library_panel.tab = tab;
    if tab == Tab::Plok {
        plok_panel::on_tab_open(app);
    }
}

pub(crate) fn draw(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let screen = ui.ctx().content_rect();
    let inner = 2.0 * (MARGIN + f32::from(FRAME_MARGIN));
    let content_size = egui::vec2(
        (screen.width() - inner).max(0.0),
        (screen.height() - inner).max(0.0),
    );
    let frame = egui::Frame::new()
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(FRAME_MARGIN));
    let modal = egui::Modal::new(egui::Id::new("drums_kit_library"))
        .backdrop_color(egui::Color32::from_black_alpha(180))
        .frame(frame);
    let response = modal.show(ui.ctx(), |ui| {
        ui.set_width(content_size.x);
        ui.set_height(content_size.y);
        draw_contents(ui, app);
    });
    probe(ui, "library.panel", response.response.rect);

    // Esc closes, as does Close; a click on the backdrop does not — a
    // stray click outside a panel this size is far likelier a miss than a
    // request to dismiss it, and `should_close` would count it. Esc while
    // typing in a field, or with a menu open, is left to those: egui has
    // already taken the field's focus away by now, hence "had focus".
    // With a delete armed, Esc backs out of that first.
    let escape = response.is_top_modal
        && !response.any_popup_open
        && !library_ui::text_field_had_focus(ui.ctx())
        && ui
            .ctx()
            .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if escape {
        if app.browser.pending_delete().is_some() {
            app.browser.cancel_delete();
        } else {
            close(app);
        }
    }
}

fn draw_contents(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("KIT LIBRARY")
                .strong()
                .color(theme::ACCENT)
                .size(14.0),
        );
        ui.add_space(10.0);
        let current = match app.library_panel.tab {
            Tab::Installed => 0,
            Tab::Plok => 1,
        };
        if let Some(i) =
            widgets::segmented(ui, &[Tab::Installed.label(), Tab::Plok.label()], current)
        {
            set_tab(app, if i == 0 { Tab::Installed } else { Tab::Plok });
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Close").clicked() {
                close(app);
            }
            ui.add_space(8.0);
            let lib = app.library.read();
            let n = lib.len();
            ui.label(
                egui::RichText::new(format!(
                    "{n} kit{} · {}",
                    plural(n),
                    format_bytes(lib.total_bytes())
                ))
                .size(11.0)
                .color(theme::TEXT_DIM),
            );
        });
    });
    ui.add_space(6.0);
    if !app.library_panel.open {
        return;
    }
    match app.library_panel.tab {
        Tab::Installed => draw_installed(ui, app),
        Tab::Plok => plok_panel::draw_tab(ui, app),
    }
}

fn draw_installed(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    app.refresh_rows();
    if app.rows.rows.is_empty() {
        draw_empty_state(ui, app);
        return;
    }

    // Search, switches, facets, sort. Wrapped: at the editor's minimum
    // width the row does not fit on one line.
    ui.horizontal_wrapped(|ui| {
        let search =
            library_ui::search_field(ui, &mut app.browser, "search name, tag, mic, piece…", 190.0);
        probe(ui, "library.search", search.rect);
        if widgets::chip_button(ui, "★ only", app.browser.favorites_only()) {
            let on = !app.browser.favorites_only();
            app.browser.set_favorites_only(on);
        }
        if widgets::chip_button(ui, "Recent", app.browser.recent_only()) {
            let on = !app.browser.recent_only();
            app.browser.set_recent_only(on);
        }
        for (facet, label) in FACETS {
            library_ui::facet_menu(
                ui,
                ("drums_lib_facet", *facet),
                label,
                &mut app.browser,
                &app.rows,
                facet,
            );
        }
        let options = sort_options();
        let current = options
            .iter()
            .find(|(_, k)| *k == app.browser.sort().key)
            .map(|(l, _)| *l)
            .unwrap_or("Slot");
        egui::ComboBox::from_id_salt("drums_lib_sort")
            .selected_text(format!("Sort: {current}"))
            .show_ui(ui, |ui| {
                for (label, key) in options {
                    if ui
                        .selectable_label(app.browser.sort().key == key, label)
                        .clicked()
                    {
                        app.browser.set_sort(Sort::by(key));
                    }
                }
            });
    });
    ui.add_space(4.0);
    app.refresh_rows();

    // Pinned first, from the bottom up, so they claim their height before
    // the list and the detail facts share the rest.
    egui::Panel::bottom("drums_lib_footer")
        .frame(egui::Frame::NONE)
        .resizable(false)
        .show_separator_line(false)
        .show_inside(ui, |ui| {
            ui.separator();
            draw_footer(ui, app);
        });
    egui::Panel::bottom("drums_lib_actions")
        .frame(egui::Frame::NONE)
        .resizable(false)
        .show_separator_line(false)
        .show_inside(ui, |ui| draw_actions(ui, app));

    let detail_h = (ui.available_height() * 0.45).clamp(60.0, 200.0);
    let list_h = (ui.available_height() - detail_h - 12.0).max(44.0);
    let loaded_key = app.loaded_entry().map(|e| e.mark_key());
    let rows = &app.rows;
    let is_error = |row: usize| {
        matches!(
            rows.rows[row].entry.status,
            EntryStatus::ManifestError(_) | EntryStatus::MissingFiles(_)
        )
    };
    let resp = ui
        .allocate_ui(egui::vec2(ui.available_width(), list_h), |ui| {
            library_ui::library_list(
                ui,
                "drums_lib_list",
                &mut app.browser,
                rows,
                &ListOptions {
                    row_height: 22.0,
                    columns: &COLUMNS,
                    loaded: loaded_key.as_deref(),
                    is_error: Some(&is_error),
                    ..ListOptions::default()
                },
            )
        })
        .inner;
    if let Some(row) = resp.star_clicked {
        let id = app.rows.rows[row].entry.id.clone();
        app.toggle_favorite(&id);
    }
    if let Some(row) = resp.double_clicked {
        let entry = app.rows.rows[row].entry.clone();
        app.load_entry(&entry, LoadKind::Pick);
    }

    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("drums_lib_detail")
        .max_height(detail_h)
        .auto_shrink([false, true])
        .show(ui, |ui| draw_detail(ui, app));
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// The source as the detail pane names it.
fn source_text(source: Source) -> &'static str {
    match source {
        Source::Plok => "plok.org",
        Source::Imported => "imported",
        Source::Local => "local",
    }
}

/// "plok.org · added 2026-04-12 · 2,835 samples · 8.5 GB · /…/drumkits/Drummica"
pub(crate) fn detail_line(entry: &Entry) -> String {
    let mut parts = vec![source_text(entry.source).to_string()];
    if let Some(date) = resonance_common::library_marks::format_timestamp(entry.added_at) {
        parts.push(format!("added {}", &date[..10.min(date.len())]));
    }
    parts.push(format!(
        "{} sample{}",
        entry.sample_count,
        plural(entry.sample_count as usize)
    ));
    parts.push(
        entry
            .size_bytes
            .map(format_bytes)
            .unwrap_or_else(|| "size not measured yet".into()),
    );
    parts.push(entry.dir.display().to_string());
    parts.join(" · ")
}

/// A labelled, wrapping line of the detail pane.
fn fact(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(label).size(11.0).color(theme::TEXT_DIM));
        ui.label(egui::RichText::new(value).size(11.0).color(theme::TEXT_2));
    });
}

fn draw_detail(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let Some(row) = app.browser.selected_row() else {
        ui.label(
            egui::RichText::new("Select a kit to see its details. Double-click loads it.")
                .color(theme::TEXT_DIM)
                .size(11.0),
        );
        return;
    };
    let entry = app.rows.rows[row].entry.clone();
    let shown = ui.vertical(|ui| {
        ui.label(
            egui::RichText::new(&entry.name)
                .strong()
                .size(13.0)
                .color(theme::TEXT),
        );
        ui.add(
            egui::Label::new(
                egui::RichText::new(detail_line(&entry))
                    .size(11.0)
                    .color(theme::TEXT_DIM),
            )
            .wrap(),
        );
        if let Some(desc) = entry.description() {
            ui.label(egui::RichText::new(desc).size(11.0).color(theme::TEXT_2));
        }
        match &entry.status {
            EntryStatus::Ok => {}
            EntryStatus::ManifestError(reason) => {
                warn(ui, &format!("manifest error: {reason}"));
            }
            EntryStatus::MissingFiles(n) => {
                warn(ui, &format!("{n} sample file{} missing", plural(*n)))
            }
            EntryStatus::DuplicateOf(dir) => {
                warn(ui, &format!("the same kit as {}", dir.display()))
            }
        }
        let pieces: Vec<&str> = entry.pieces.iter().map(|p| p.name.as_str()).collect();
        fact(
            ui,
            "pieces:",
            &format!("{} ({})", pieces.join(", "), pieces.len()),
        );
        let mics: Vec<String> = entry.mic_setups.values().map(|m| m.label()).collect();
        fact(
            ui,
            "mics:",
            &if mics.is_empty() {
                "—".to_string()
            } else {
                mics.join(", ")
            },
        );
        let artics: Vec<String> = entry
            .articulations
            .iter()
            .map(|a| {
                if a.label.is_empty() {
                    format!(
                        "{} / {}",
                        entry.piece_name(&a.primary),
                        entry.piece_name(&a.alt)
                    )
                } else {
                    a.label.clone()
                }
            })
            .collect();
        fact(
            ui,
            "articulations:",
            &if artics.is_empty() {
                "—".to_string()
            } else {
                artics.join(", ")
            },
        );
        fact(
            ui,
            "layers / RR:",
            &format!(
                "{} velocity layers · {} round robin",
                entry.layers_max, entry.rr_max
            ),
        );
        if !entry.index_tags().is_empty() {
            fact(ui, "plok.org tags:", &entry.index_tags().join(", "));
        }

        // Personal tags, with completion from every kind's tags.
        let tags = app
            .rows
            .marks_of(row)
            .map(|m| m.tags.clone())
            .unwrap_or_default();
        let suggestions = if app.tag_draft.trim().is_empty() {
            Vec::new()
        } else {
            app.library
                .marks()
                .snapshot()
                .complete_tag(&app.tag_draft, &tags, 6)
        };
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("tags:")
                    .size(11.0)
                    .color(theme::TEXT_DIM),
            );
            let r = library_ui::tag_row(
                ui,
                ("drums_lib_tags", &entry.id),
                &tags,
                &mut app.tag_draft,
                &suggestions,
            );
            let result = match (r.added, r.removed) {
                (Some(t), _) => Some(app.library.add_tag(&entry.id, &t)),
                (None, Some(t)) => Some(app.library.remove_tag(&entry.id, &t)),
                _ => None,
            };
            if let Some(Err(e)) = result {
                app.browser
                    .set_error(format!("could not save the tags: {e}"));
            }
        });
        let used = app.library.usage_count(&entry.id);
        if used > 0 {
            ui.label(
                egui::RichText::new(format!("used in {used} open drum instance{}", plural(used)))
                    .size(11.0)
                    .color(theme::TEXT_DIM),
            );
        }
    });
    probe(ui, "library.detail", shown.response.rect);
}

/// The selected kit's actions — `[Load] [Reveal] [Re-download]
/// [Delete…]` — and the delete confirm, pinned under the detail facts.
fn draw_actions(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let Some(row) = app.browser.selected_row() else {
        return;
    };
    let entry = app.rows.rows[row].entry.clone();
    let key = app.rows.rows[row].key.clone();
    ui.add_space(4.0);
    let shown = ui.horizontal_wrapped(|ui| {
        let b = ui.add_enabled(entry.is_loadable(), egui::Button::new("Load"));
        probe(ui, "library.action.load", b.rect);
        if b.clicked() {
            app.load_entry(&entry, LoadKind::Pick);
        }
        let b = ui.button("Reveal");
        probe(ui, "library.action.reveal", b.rect);
        if b.clicked() {
            if let Err(e) = resonance_common::reveal::reveal(&entry.dir) {
                app.browser
                    .set_error(format!("could not open the file manager: {e}"));
            }
        }
        if let Some(kit) = redownload_kit(app, &entry) {
            let busy = app.library.download().state.lock().is_working_on(&kit.name);
            let b = ui
                .add_enabled(!busy, egui::Button::new("Re-download"))
                .on_hover_text("Download this kit from plok.org again and replace it in place");
            probe(ui, "library.action.redownload", b.rect);
            if b.clicked() {
                app.my_downloads.insert(kit.name.clone());
                app.browser
                    .set_info(format!("re-downloading \"{}\"…", entry.name));
                app.library.download().send(Command::Redownload {
                    kit,
                    existing_dir: entry.dir.clone(),
                });
            }
        }
        // Arming does no I/O, so it is never disabled; the confirm is
        // what waits for a running job.
        if app.browser.pending_delete() != Some(key.as_str()) {
            let b = ui.button("Delete…");
            probe(ui, "library.action.delete", b.rect);
            if b.clicked() {
                app.browser.begin_delete(key.clone());
                // The pinned panel is sized from its last frame's
                // content; lay this frame out again with the confirm in
                // it, or its last line is clipped for a frame.
                ui.ctx()
                    .request_discard("the delete confirm grew the action row");
            }
        }
    });
    probe(ui, "library.actions", shown.response.rect);
    draw_delete_confirm(ui, app, &key, &entry);
    ui.add_space(4.0);
}

fn warn(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(11.0).color(theme::DANGER));
}

/// What a Re-download of `entry` fetches: only for a kit that came from
/// plok.org and says which file it was. The fresh index's entry when it
/// has one (its sha256 is the one to check against), else the sidecar's.
fn redownload_kit(app: &DrumsEditorApp, entry: &Entry) -> Option<ServerKit> {
    if entry.source != Source::Plok {
        return None;
    }
    let sidecar = entry.sidecar.as_ref()?;
    let file = sidecar.index_file.clone()?;
    let name = sidecar
        .index_name
        .clone()
        .unwrap_or_else(|| entry.name.clone());
    let from_index = app
        .library
        .download()
        .state
        .lock()
        .index
        .as_ref()
        .and_then(|i| i.find(&name).cloned());
    Some(from_index.unwrap_or_else(|| ServerKit {
        description: sidecar.description.clone(),
        tags: sidecar.index_tags.clone(),
        ..ServerKit::new(name, file)
    }))
}

/// The confirm-in-place line of §3.5: `Delete "Drummica" (8.5 GB)?
/// [Delete] [Cancel]`, saying who is still using it. Shown only for the
/// row that was armed, and the key it hands back — not the displayed
/// entry — is what gets deleted.
fn draw_delete_confirm(ui: &mut egui::Ui, app: &mut DrumsEditorApp, key: &str, entry: &Entry) {
    let used = app.library.usage_count(&entry.id);
    let mut detail = String::from(
        "The folder is removed from disk. Projects that use it will show it as missing. \
         Favourites and tags are kept for 90 days.",
    );
    if used > 0 {
        detail = format!(
            "Used by {used} open drum instance{}; they keep playing until reloaded. {detail}",
            plural(used)
        );
    }
    let size = entry
        .size_bytes
        .map(format_bytes)
        .unwrap_or_else(|| "size unknown".into());
    let prompt = format!(
        "Delete \"{}\" ({size})?",
        elide(&entry.name, PROMPT_NAME_CHARS)
    );
    // A scan, import or delete holds the library: the confirm waits for
    // it rather than racing it (and losing — a delete the library refuses
    // as busy used to be dropped with the confirm already spent).
    let blocked = app.jobs.writing().then(|| {
        format!(
            "waiting for {} to finish",
            app.jobs
                .label()
                .unwrap_or("the library")
                .trim_end_matches('…')
        )
    });
    if let ConfirmOutcome::Confirmed(confirmed) = library_ui::confirm_delete_row_blocked(
        ui,
        &mut app.browser,
        key,
        &prompt,
        Some(&detail),
        blocked.as_deref(),
    ) {
        start_delete(app, &confirmed);
    }
}

/// How much of a kit's name the delete prompt shows before eliding it.
const PROMPT_NAME_CHARS: usize = 40;

/// `text`, cut to `max` characters with "…" when longer.
fn elide(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max.saturating_sub(1)).collect();
    s.push('…');
    s
}

/// Delete the kit whose row key is `key`, as a job — or, while a job is
/// running, once it finishes. The library rescans (freeing its slot) and
/// the marks orphan pass stamps its marks.
pub(crate) fn start_delete(app: &mut DrumsEditorApp, key: &str) {
    if app.jobs.busy() {
        app.run_or_queue(Queued::Delete(key.to_string()));
        return;
    }
    let Some(row) = app.rows.rows.iter().find(|r| r.key == key) else {
        app.browser
            .set_error("that kit is no longer in the library");
        return;
    };
    let (dir, name) = (row.entry.dir.clone(), row.entry.name.clone());
    let view_pos = app.browser.position_in_view(key);
    let library = app.library.clone();
    let started = app.jobs.start(
        JobKind::Delete,
        format!("deleting \"{name}\"…"),
        false,
        move |_| {
            let result = match library.delete(&dir) {
                None => Err(format!("could not delete \"{name}\": {BUSY}")),
                Some(Ok(_)) => Ok(()),
                Some(Err(e)) => Err(format!("could not delete \"{name}\": {e}")),
            };
            JobDone::Deleted {
                name,
                result,
                view_pos,
            }
        },
    );
    if !started {
        app.browser
            .set_error(format!("could not start the delete: {BUSY}"));
    }
}

/// Why the library refused a write: another writer holds it — a download
/// installing, or another drum editor's import, delete or scan. Not only
/// "installing a kit", as this used to say.
pub(crate) const BUSY_WHY: &str =
    "the library is busy (a download or another drum editor is writing to it).";

const BUSY: &str =
    "the library is busy (a download or another drum editor is writing to it); try again in a moment";

fn draw_footer(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let shown = ui.horizontal(|ui| {
        import_button(ui, app);
        let b = ui.add_enabled(!app.jobs.writing(), egui::Button::new("Rescan"));
        probe(ui, "library.rescan", b.rect);
        if b.clicked() {
            app.run_or_queue(Queued::Rescan);
        }
        draw_job_status(ui, app);
    });
    probe(ui, "library.footer", shown.response.rect);
}

/// `[Import kit folder / .zip… ▾]`: a folder or a zip, picked in a dialog
/// on its own thread, then copied in as a job.
fn import_button(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let enabled = !app.jobs.writing() && !app.picker.busy();
    ui.add_enabled_ui(enabled, |ui| {
        ui.menu_button("Import kit folder / .zip…", |ui| {
            if ui.button("A kit folder…").clicked() {
                app.picker.start(PickKind::Folder);
                ui.close();
            }
            if ui.button("A .zip of a kit…").clicked() {
                app.picker.start(PickKind::Zip);
                ui.close();
            }
        });
    });
}

/// The running job (with progress and Cancel for an import), else the
/// last notice.
fn draw_job_status(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    if app.picker.busy() {
        ui.label(
            egui::RichText::new("choose a kit to import…")
                .size(11.0)
                .color(theme::TEXT_DIM),
        );
        return;
    }
    if let Some(mut label) = app.jobs.label().map(str::to_string) {
        let waiting = app.queued.len();
        if waiting > 0 {
            label.push_str(&format!(" · {waiting} waiting"));
        }
        ui.label(egui::RichText::new(label).size(11.0).color(theme::TEXT_DIM));
        if app.jobs.cancellable() {
            if let Some(p) = app.jobs.progress() {
                let fraction = if p.bytes_total > 0 {
                    p.bytes_done as f32 / p.bytes_total as f32
                } else {
                    0.0
                };
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_width(140.0)
                        .text(format!(
                            "{} / {}",
                            format_bytes(p.bytes_done),
                            format_bytes(p.bytes_total)
                        )),
                );
            }
            if ui.button("Cancel").clicked() {
                app.jobs.cancel();
            }
        }
        return;
    }
    if let Some(n) = app.browser.notice() {
        let color = if n.is_error() {
            theme::DANGER
        } else {
            theme::TEXT_DIM
        };
        // One line, elided: the full text — a long error is the part the
        // user needs — is on hover.
        let text = n.text().to_string();
        let l = ui
            .add(egui::Label::new(egui::RichText::new(&text).size(11.0).color(color)).truncate())
            .on_hover_text(text);
        probe(ui, "library.notice", l.rect);
    }
}

/// Copy `src` (a kit folder or a `.zip`) into the library, as a job with
/// progress and Cancel — or, while a job is running, once it finishes. A
/// cancel leaves nothing behind. `false` when it could not start (the
/// browser says why), so nothing will report back.
pub(crate) fn start_import(app: &mut DrumsEditorApp, src: PathBuf) -> bool {
    if app.jobs.busy() {
        app.run_or_queue(Queued::Import(src));
        return true;
    }
    let library = app.library.clone();
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "kit".into());
    let started = app.jobs.start(
        JobKind::Import,
        format!("importing \"{name}\"…"),
        true,
        move |ctx| {
            let progress = ctx.progress.clone();
            let job = ImportJob::new()
                .cancel(&ctx.cancel)
                .progress(move |p| *progress.lock() = Some(p));
            JobDone::Imported(Box::new(match library.import(&src, job) {
                None => Err(BUSY.to_string()),
                Some(Ok(outcome)) => Ok(outcome),
                Some(Err(LibraryError::Cancelled)) => {
                    Err(format!("import of \"{name}\" cancelled"))
                }
                Some(Err(e)) => Err(format!("could not import \"{name}\": {e}")),
            }))
        },
    );
    if !started {
        app.browser
            .set_error(format!("could not start the import: {BUSY}"));
    }
    started
}

fn draw_empty_state(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new("No kits installed yet.")
                .size(14.0)
                .color(theme::TEXT),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Browse plok.org").clicked() {
                set_tab(app, Tab::Plok);
            }
            import_button(ui, app);
        });
        ui.add_space(8.0);
        draw_job_status(ui, app);
        if let Some(root) = app.library.root().map(|r| r.to_path_buf()) {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("Kits live in {}", root.display()))
                        .size(11.0)
                        .color(theme::TEXT_DIM),
                );
                if ui.button("Reveal").clicked() {
                    let _ = std::fs::create_dir_all(&root);
                    if let Err(e) = resonance_common::reveal::reveal(&root) {
                        app.browser
                            .set_error(format!("could not open the file manager: {e}"));
                    }
                }
            });
        }
    });
}
