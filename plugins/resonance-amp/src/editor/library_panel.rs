//! The Library overlay (nam-model-library.md §6.2): a full-window panel
//! over a dimmed editor with two tabs, `Installed | Tone3000`.
//!
//! ```text
//! ┌ LIBRARY  [ Installed | Tone3000 ]                 41 models · 612 MB   [Close] ┐
//! │ [search name, author, gear, tag…]  (★ only) (Recent)  Gear  Type  Arch  Sort    │
//! │ ★ Friedman BE-100 · standard   J. Smith  Friedman BE-100  amp  crunch  A2  48k  4.1 MB │
//! │ …                                                                               │
//! │ Friedman BE-100 · standard                                                      │
//! │ by J. Smith · Tone3000 tone #1934 · A2 WaveNet · 48 kHz · ESR 0.0041 · added …  │
//! │ tags: (rhythm ×) (djent ×) [+ tag]         used in 2 open amps                  │
//! │ [Load]  [Reveal]  [Re-download]  [Delete…]                                      │
//! │ [Import .nam…]  [Rescan]                                    status / last error │
//! └─────────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! The behaviour lives in the shared `BrowserModel` and `nam_rows`; this
//! file only turns clicks into their calls. Anything that hashes, copies
//! or waits on the library's writer lock is a job (`jobs.rs`).

use plugin_gui_core::egui;
use plugin_gui_core::widgets::{chip_button, segmented};
use resonance_common::nam_library::{Entry, EntryStatus, Source};
use resonance_plugin::library_ui::{self, ColumnSpec, ConfirmOutcome, ListOptions};
use resonance_plugin::library_view::Sort;

use super::jobs::JobDone;
use super::{actions, theme, tone3000_panel, AmpEditorApp};
use crate::library_rows::{format_size, sort_options, FACETS};

/// Which tab is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Tab {
    #[default]
    Installed,
    Tone3000,
}

/// The overlay's editor-only state. Not persisted.
#[derive(Default)]
pub(crate) struct LibraryPanelState {
    pub(crate) open: bool,
    pub(crate) tab: Tab,
    /// The last download of THIS editor the panel has reacted to.
    pub(crate) seen_download: Option<std::path::PathBuf>,
}

/// The Installed tab's columns: author, gear, gear type, tone type,
/// architecture, sample rate, size.
const COLUMNS: [ColumnSpec; 7] = [
    ColumnSpec::left(90.0),
    ColumnSpec::left(130.0),
    ColumnSpec::left(56.0),
    ColumnSpec::left(56.0),
    ColumnSpec::left(30.0),
    ColumnSpec::right(38.0),
    ColumnSpec::right(56.0),
];

/// Open the overlay: on Installed, or on Tone3000 when nothing is installed.
pub(crate) fn open(app: &mut AmpEditorApp) {
    app.library_panel.open = true;
    let empty = app.params.library.read().is_empty();
    app.library_panel.tab = if empty { Tab::Tone3000 } else { Tab::Installed };
}

pub(crate) fn draw(ui: &mut egui::Ui, app: &mut AmpEditorApp) {
    let screen = ui.ctx().content_rect();
    ui.painter()
        .rect_filled(screen, 0.0, egui::Color32::from_black_alpha(180));
    let rect = screen.shrink(24.0);
    egui::Area::new(egui::Id::new("amp_library_panel"))
        .fixed_pos(rect.min)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(theme::PANEL)
                .stroke(egui::Stroke::new(1.0, theme::BORDER))
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(rect.width() - 24.0);
                    ui.set_height(rect.height() - 24.0);
                    draw_contents(ui, app);
                });
        });
    // Esc closes the overlay — but not while a text field (search, a tag)
    // is being typed in: there it only leaves the field (egui drops the
    // focus before this runs, hence "had focus").
    if !library_ui::text_field_had_focus(ui.ctx()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.library_panel.open = false;
    }
}

fn draw_contents(ui: &mut egui::Ui, app: &mut AmpEditorApp) {
    // Polled here (not in `draw_footer`) because `draw_installed` skips
    // the footer entirely for an empty library (`draw_empty_state`
    // returns early) — but its own "Import .nam…" can still have a
    // dialog in flight that needs collecting every frame regardless of
    // which state the panel is in.
    #[cfg(not(target_os = "macos"))]
    if let Some(files) = actions::poll_nam_picker(ui.ctx(), actions::import_nam_picker_id()) {
        actions::start_import(app, files);
    }

    // After a download this editor asked for, switch to Installed with the
    // new row selected. (The Tone3000 worker is shared by every amp's
    // editor, so its own `last_downloaded` is not this editor's.)
    let downloaded = app.my_download.lock().clone();
    if downloaded.is_some() && downloaded != app.library_panel.seen_download {
        app.library_panel.seen_download = downloaded.clone();
        app.refresh_rows();
        if let Some(path) = downloaded {
            if let Some(row) = app.rows.rows.iter().find(|r| r.entry.path == path) {
                let key = row.key.clone();
                app.browser.select(key);
                app.library_panel.tab = Tab::Installed;
            }
        }
    }

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("LIBRARY")
                .strong()
                .color(theme::ACCENT)
                .size(14.0),
        );
        ui.add_space(10.0);
        let current = match app.library_panel.tab {
            Tab::Installed => 0,
            Tab::Tone3000 => 1,
        };
        if let Some(i) = segmented(ui, &["Installed", "Tone3000"], current) {
            app.library_panel.tab = if i == 0 { Tab::Installed } else { Tab::Tone3000 };
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Close").clicked() {
                app.library_panel.open = false;
            }
            ui.add_space(8.0);
            let lib = app.params.library.read();
            ui.label(
                egui::RichText::new(format!(
                    "{} models · {}",
                    lib.len(),
                    format_size(lib.total_bytes())
                ))
                .size(11.0)
                .color(theme::TEXT_DIM),
            );
        });
    });
    ui.add_space(6.0);

    match app.library_panel.tab {
        Tab::Installed => draw_installed(ui, app),
        Tab::Tone3000 => {
            let done = actions::download_done(app);
            let picked = {
                let library = app.params.library.read();
                tone3000_panel::draw_tab(ui, &mut app.tone3000_panel, &app.tone3000, &library, &done)
            };
            if let Some(tone3000_panel::ModelRowAction::Load { slot, .. }) = picked {
                match slot {
                    Some(slot) => actions::load_slot(app, slot, actions::LoadKind::Pick),
                    None => app.browser.set_error("That model has no slot to load it through"),
                }
            }
        }
    }
}

fn draw_installed(ui: &mut egui::Ui, app: &mut AmpEditorApp) {
    app.refresh_rows();
    if app.rows.rows.is_empty() {
        draw_empty_state(ui, app);
        return;
    }

    // Search, switches, facets, sort. Wrapped: at the editor's 760 px
    // minimum the row does not fit on one line.
    ui.horizontal_wrapped(|ui| {
        library_ui::search_field(ui, &mut app.browser, "search name, author, gear, tag…", 200.0);
        if chip_button(ui, "★ only", app.browser.favorites_only()) {
            let on = !app.browser.favorites_only();
            app.browser.set_favorites_only(on);
        }
        if chip_button(ui, "Recent", app.browser.recent_only()) {
            let on = !app.browser.recent_only();
            app.browser.set_recent_only(on);
        }
        for (facet, label) in FACETS {
            library_ui::facet_menu(
                ui,
                ("amp_lib_facet", *facet),
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
        egui::ComboBox::from_id_salt("amp_lib_sort")
            .selected_text(format!("Sort: {current}"))
            .show_ui(ui, |ui| {
                for (label, key) in options {
                    if ui.selectable_label(app.browser.sort().key == key, label).clicked() {
                        app.browser.set_sort(Sort::by(key));
                    }
                }
            });
    });
    ui.add_space(4.0);
    app.refresh_rows();

    let detail_h = 150.0;
    let footer_h = 28.0;
    let list_h = (ui.available_height() - detail_h - footer_h - 12.0).max(80.0);
    let loaded_key = app
        .params
        .status
        .lock()
        .id
        .as_deref()
        .map(resonance_common::nam_library::mark_key);
    let rows = &app.rows;
    let is_error = |row: usize| matches!(rows.rows[row].entry.status, EntryStatus::Unreadable(_));
    let resp = ui
        .allocate_ui(egui::vec2(ui.available_width(), list_h), |ui| {
            library_ui::library_list(
                ui,
                "amp_lib_list",
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
        load_row(app, row);
    }

    ui.separator();
    draw_detail(ui, app, detail_h);
    ui.separator();
    draw_footer(ui, app);
}

fn load_row(app: &mut AmpEditorApp, row: usize) {
    let entry = app.rows.rows[row].entry.clone();
    if let EntryStatus::Unreadable(reason) = &entry.status {
        app.browser.set_error(format!("\"{}\" cannot be loaded: {reason}", entry.name));
        return;
    }
    if let Err(e) = actions::load_entry(app, &entry) {
        app.browser.set_error(e);
    }
}

fn draw_detail(ui: &mut egui::Ui, app: &mut AmpEditorApp, height: f32) {
    let Some(row) = app.browser.selected_row() else {
        ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
            ui.label(
                egui::RichText::new("Select a model to see its details. Double-click loads it.")
                    .color(theme::TEXT_DIM)
                    .size(11.0),
            );
        });
        return;
    };
    let entry = app.rows.rows[row].entry.clone();
    let key = app.rows.rows[row].key.clone();
    ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(&entry.name).strong().size(13.0).color(theme::TEXT));
            ui.label(
                egui::RichText::new(detail_line(&entry))
                    .size(11.0)
                    .color(theme::TEXT_DIM),
            );
            if let EntryStatus::Unreadable(reason) = &entry.status {
                ui.label(
                    egui::RichText::new(format!("unreadable: {reason}"))
                        .size(11.0)
                        .color(theme::DANGER),
                );
            }
            // Personal tags, with completion from every kind's tags and the
            // seeded vocabulary (library_marks).
            let tags = app.rows.marks_of(row).map(|m| m.tags.clone()).unwrap_or_default();
            let suggestions = if app.tag_draft.trim().is_empty() {
                Vec::new()
            } else {
                app.params
                    .library
                    .marks()
                    .snapshot()
                    .complete_tag(&app.tag_draft, &tags, 6)
            };
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("tags:").size(11.0).color(theme::TEXT_DIM));
                let r = library_ui::tag_row(
                    ui,
                    ("amp_lib_tags", &entry.id),
                    &tags,
                    &mut app.tag_draft,
                    &suggestions,
                );
                let result = match (r.added, r.removed) {
                    (Some(t), _) => Some(app.params.library.add_tag(&entry.id, &t)),
                    (None, Some(t)) => Some(app.params.library.remove_tag(&entry.id, &t)),
                    _ => None,
                };
                if let Some(Err(e)) = result {
                    app.browser.set_error(format!("could not save the tags: {e}"));
                }
            });
            let used = app.usage_count(&entry.id);
            if used > 0 {
                ui.label(
                    egui::RichText::new(format!("used in {used} open amp{}", plural(used)))
                        .size(11.0)
                        .color(theme::TEXT_DIM),
                );
            }
            ui.horizontal(|ui| {
                let loadable = entry.slot.is_some() && entry.is_ok();
                if ui.add_enabled(loadable, egui::Button::new("Load")).clicked() {
                    load_row(app, row);
                }
                if ui.button("Reveal").clicked() {
                    if let Err(e) = resonance_common::reveal::reveal(&entry.path) {
                        app.browser.set_error(format!("could not open the file manager: {e}"));
                    }
                }
                if let Source::Tone3000 { tone_id, model_id } = entry.source {
                    if actions::tone3000_connected(app) {
                        if ui.button("Re-download").clicked() {
                            // Refreshes the library's file; does not switch
                            // this amp to it.
                            actions::redownload_only(app, &entry, tone_id, model_id);
                            app.browser.set_info(format!("re-downloading \"{}\"…", entry.name));
                        }
                    } else if ui.button("Connect… to re-download").clicked() {
                        app.tone3000.send(crate::tone3000::worker::Command::Authenticate);
                    }
                }
                if app.browser.pending_delete() != Some(key.as_str())
                    && ui
                        .add_enabled(!app.jobs.busy(), egui::Button::new("Delete…"))
                        .clicked()
                {
                    app.browser.begin_delete(key.clone());
                }
            });
            draw_delete_confirm(ui, app, &key, &entry);
        });
    });
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// The confirm-in-place line of §7.1: "Delete "<name>" (4.1 MB)? [Delete]
/// [Cancel]", saying who is still using it and what happens to projects.
/// Shown only for the row that was armed, and the key it hands back —
/// not the displayed entry — is what gets deleted.
fn draw_delete_confirm(ui: &mut egui::Ui, app: &mut AmpEditorApp, key: &str, entry: &Entry) {
    let used = app.usage_count(&entry.id);
    let mut detail = String::from("Projects that use it will show it as missing.");
    if used > 0 {
        detail = format!(
            "Used by {used} open amp{} — they keep playing until reloaded. {detail}",
            plural(used)
        );
    }
    let prompt = format!("Delete \"{}\" ({})?", entry.name, format_size(entry.size_bytes));
    if let ConfirmOutcome::Confirmed(confirmed) =
        library_ui::confirm_delete_row(ui, &mut app.browser, key, &prompt, Some(&detail))
    {
        start_delete(app, &confirmed);
    }
}

/// Delete the entry whose row key is `key` (a real delete, D4), as a job.
/// Its slot is freed under the no-reuse rule; its marks are kept for the
/// orphan window, so a re-download or re-import keeps the star and tags.
pub(crate) fn start_delete(app: &mut AmpEditorApp, key: &str) {
    let Some(row) = app.rows.rows.iter().find(|r| r.key == key) else {
        app.browser.set_error("that model is no longer in the library");
        return;
    };
    let (path, name) = (row.entry.path.clone(), row.entry.name.clone());
    let library = app.params.library.clone();
    let started = app.jobs.start("deleting…", move || JobDone::Deleted {
        name,
        result: library
            .mutate(|lib| lib.delete(&path))
            .map(|_| ())
            .map_err(|e| e.to_string()),
    });
    if !started {
        app.browser.set_error("the library is busy; try again in a moment");
    }
}

/// "by J. Smith · Tone3000 tone #1934 · WaveNet A2 · 48 kHz · ESR 0.0041 · added 2026-09-12"
pub(crate) fn detail_line(entry: &Entry) -> String {
    let mut parts = Vec::new();
    if let Some(a) = &entry.author {
        parts.push(format!("by {a}"));
    }
    parts.push(match &entry.source {
        Source::Tone3000 { tone_id, .. } => format!("Tone3000 tone #{tone_id}"),
        Source::Imported => "imported".into(),
        Source::External => "external".into(),
    });
    if let Some(g) = &entry.gear {
        parts.push(g.clone());
    }
    parts.push(entry.architecture.clone());
    parts.push(super::header::format_khz(entry.sample_rate as f32));
    if let Some(esr) = entry.esr {
        parts.push(format!("ESR {esr:.4}"));
    }
    parts.push(format_size(entry.size_bytes));
    if let Some(date) = resonance_common::library_marks::format_timestamp(entry.added_at) {
        parts.push(format!("added {}", &date[..10.min(date.len())]));
    }
    parts.join(" · ")
}

fn draw_footer(ui: &mut egui::Ui, app: &mut AmpEditorApp) {
    ui.horizontal(|ui| {
        let busy = app.jobs.busy();
        if ui.add_enabled(!busy, egui::Button::new("Import .nam…")).clicked() {
            import_clicked(ui.ctx(), app);
        }
        if ui.add_enabled(!busy, egui::Button::new("Rescan")).clicked() {
            app.start_rescan();
        }
        if let Some(label) = app.jobs.label() {
            ui.label(egui::RichText::new(label).size(11.0).color(theme::TEXT_DIM));
        } else if let Some(n) = app.browser.notice() {
            let color = if n.is_error() { theme::DANGER } else { theme::TEXT_DIM };
            ui.label(egui::RichText::new(n.text()).size(11.0).color(color));
        }
    });
}

/// Import one or more files (a job); a single file is also loaded, and a
/// file that is already in the library selects its row.
pub(crate) fn import_clicked(ctx: &egui::Context, app: &mut AmpEditorApp) {
    #[cfg(target_os = "macos")]
    {
        let files = actions::pick_nam_files(true);
        actions::start_import(app, files);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        actions::start_nam_picker(ctx, actions::import_nam_picker_id(), true);
    }
}

fn draw_empty_state(ui: &mut egui::Ui, app: &mut AmpEditorApp) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new("No models installed yet.").size(14.0).color(theme::TEXT));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Browse Tone3000").clicked() {
                app.library_panel.tab = Tab::Tone3000;
            }
            if ui.button("Import .nam…").clicked() {
                import_clicked(ui.ctx(), app);
            }
        });
        ui.add_space(8.0);
        if let Some(root) = app.params.library.root().map(|r| r.to_path_buf()) {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("Models live in {}", root.display()))
                        .size(11.0)
                        .color(theme::TEXT_DIM),
                );
                if ui.button("Reveal").clicked() {
                    let _ = std::fs::create_dir_all(&root);
                    if let Err(e) = resonance_common::reveal::reveal(&root) {
                        app.browser.set_error(format!("could not open the file manager: {e}"));
                    }
                }
            });
        }
    });
}
