//! The shared preset bar for plugin editors — ◀ ☆ picker • ▶, Browse,
//! Save, Save as… — and the preset browser overlay and metadata form it
//! opens (plugin-preset-library.md §6.2–§6.5).
//!
//! Every plugin editor draws its own preset combo today, four of them
//! hardcoding "— select —" and none of them able to save (audit findings
//! X1 and X2). This is the one widget they move onto. It is deliberately
//! a thin skin: all the behaviour lives in
//! [`PresetEditor`](crate::presets::PresetEditor) and
//! [`PresetSession`](crate::presets::PresetSession), which are
//! GUI-agnostic and unit-tested, so this file only turns clicks into
//! their method calls.
//!
//! ```ignore
//! let event = preset_ui::preset_bar(
//!     ui, "gate_presets", &mut self.preset_editor, &self.bank,
//!     &self.presets, &self.params.refs(), "— preset —",
//! );
//! if matches!(event, PresetEvent::Loaded(_)) {
//!     self.push_all_params_to_host();
//! }
//! ```
//!
//! Feature-gated behind `editor-widgets` with the rest of the egui
//! helpers, so DSP-only consumers don't pull in the GUI stack.

use plugin_gui_core::egui;
use plugin_gui_core::theme::lavender as theme;
use plugin_gui_core::widgets::{star_toggle, tag_pill};

use crate::library_ui::{self, ColumnSpec, ConfirmOutcome, ListOptions};
use crate::library_view::{Sort, SortKey, TAGS_FACET};
use crate::param::Param;
use crate::presets::{FormMode, MetaForm};
use crate::presets::{
    NamingKind, PresetBank, PresetEditor, PresetEvent, PresetRecord, PresetRef, PresetSession,
    PresetSource,
};

/// How often the bar re-checks the user preset directory; see
/// [`crate::presets::BAR_REFRESH`].
pub use crate::presets::BAR_REFRESH;

/// Draw the preset bar. Returns what the user did, if anything.
pub fn preset_bar(
    ui: &mut egui::Ui,
    id_salt: &str,
    editor: &mut PresetEditor,
    bank: &PresetBank,
    session: &PresetSession,
    params: &[&dyn Param],
    placeholder: &str,
) -> PresetEvent {
    let mut event = PresetEvent::None;

    ui.horizontal(|ui| {
        if editor.naming().is_some() {
            event = name_entry_row(ui, editor, bank, session, params);
            return;
        }

        // A project saved before preset ids carries a name-only identity;
        // give it its id now that a bank is at hand (free once resolved).
        session.resolve(bank);
        let current = session.current();
        let records = bank.records_cached(BAR_REFRESH);

        // Step through the list without opening the combo — the fastest
        // way to audition a bank. Wavetable had these as a private fork
        // around its own combo; ba todo #1280 folds them in here so the
        // whole fleet gets them. Disabled at the ends, since stepping
        // clamps rather than wraps.
        let all = &records;
        let at = current
            .as_ref()
            .and_then(|c| all.iter().position(|r| r.preset.matches(c)));
        let can_prev = !all.is_empty() && at.map(|i| i > 0).unwrap_or(true);
        let can_next = !all.is_empty() && at.map(|i| i + 1 < all.len()).unwrap_or(true);
        if ui
            .add_enabled(can_prev, egui::Button::new("◀").small().frame(false))
            .on_hover_text("Previous preset")
            .clicked()
        {
            event = editor.step(bank, session, -1, params);
        }

        // ☆/★ on the loaded preset (factory ones too: a mark never touches
        // the preset). WARM when set. Marks are per-user library state,
        // shared with every other browser.
        if let Some(c) = current.as_ref().filter(|c| c.is_resolved()) {
            bank.library().refresh_marks(BAR_REFRESH);
            let fav = bank.library().preset_marks(bank.plugin_id(), &c.id).favorite;
            let resp = plugin_gui_core::widgets::star_toggle(ui, fav).on_hover_text(if fav {
                "Unstar this preset"
            } else {
                "Star this preset"
            });
            if resp.clicked() {
                if let Err(e) = bank.library().set_favorite(bank.plugin_id(), &c.id, !fav) {
                    tracing::warn!("preset favourite: {e}");
                }
            }
        }

        let picked = picker(ui, id_salt, editor, bank, session, params, placeholder, all);
        if !matches!(picked, PresetEvent::None) {
            event = picked;
        }

        if ui
            .add_enabled(can_next, egui::Button::new("▶").small().frame(false))
            .on_hover_text("Next preset")
            .clicked()
        {
            event = editor.step(bank, session, 1, params);
        }

        // Modified is a comparison with the loaded preset (§7), re-run at
        // most every 100 ms: a knob turned and back is not an edit, a host
        // change is. The hover names what moved.
        if session.refresh_modified(params) {
            // The list of what moved is built only while hovered.
            ui.label(egui::RichText::new("•").weak()).on_hover_ui(|ui| {
                let hover = match session.changed_params(params) {
                    Some(changed) if !changed.is_empty() => {
                        let n = changed.len();
                        let shown: Vec<&str> =
                            changed.iter().take(4).map(String::as_str).collect();
                        let more = if n > 4 { ", …" } else { "" };
                        let noun = if n == 1 { "parameter" } else { "parameters" };
                        format!("{n} {noun} changed: {}{more}", shown.join(", "))
                    }
                    _ => "Edited since the preset was loaded".to_string(),
                };
                ui.label(hover);
            });
        }

        // Browse opens the library overlay; Save overwrites the loaded
        // user preset in place (factory presets are read-only: Save as…);
        // Save as… opens the metadata form. Rename and Delete live in the
        // browser (§6.2): rare, destructive, and they crowded the header.
        if ui
            .small_button("Browse")
            .on_hover_text("Search, filter and audition presets")
            .clicked()
        {
            editor.browser.open(bank, session);
            editor.browser_just_opened = true;
        }
        let is_user = current
            .as_ref()
            .is_some_and(|p| p.source == PresetSource::User && p.is_resolved());
        if ui
            .add_enabled(is_user, egui::Button::new("Save").small())
            .on_hover_text("Overwrite the loaded user preset")
            .clicked()
        {
            match session.save_in_place(bank, params) {
                Ok(p) => event = PresetEvent::Saved(p),
                Err(e) => editor.set_error(e),
            }
        }
        if ui.small_button("Save as…").clicked() {
            editor.browser.begin_save_as(bank, session);
        }
    });

    if let Some(err) = editor.error() {
        ui.label(egui::RichText::new(err).color(egui::Color32::from_rgb(0xd0, 0x60, 0x60)));
    }

    // The overlays float over the whole editor, wherever the bar sits.
    if editor.browser.open {
        let e = browser_overlay(ui.ctx(), id_salt, editor, bank, session, params);
        if !matches!(e, PresetEvent::None) {
            event = e;
        }
    }
    if editor.browser.form.is_some() {
        let e = form_overlay(ui.ctx(), id_salt, editor, bank, session, params);
        if !matches!(e, PresetEvent::None) {
            event = e;
        }
    }
    editor.browser_just_opened = false;

    // PUX-01: a preset recall from this bar — the ◀/▶ step, a combo
    // pick, a browser commit/audition/import — writes many params at
    // once through `PresetSession`, none of it through the per-gesture
    // `float_knob`/`param_knob` announce path. Without this the host's
    // mirror of those values goes stale (save→reopen can then revert
    // the recall: `project_plugin` only writes an override for a param
    // whose mirror differs from default). `request_params_rescan` just
    // asks the host to re-read; it records no undo entry of its own —
    // the recall itself is the user action worth remembering, and
    // that's on whatever triggered `PresetEvent::Loaded`, not here.
    if matches!(event, PresetEvent::Loaded(_)) {
        if let Some(announcer) = crate::editor_widgets::announcer(ui.ctx()) {
            announcer.request_params_rescan();
        }
    }

    event
}

/// The combo itself: factory bank first, then the user's own presets
/// under their own heading, so the two sets are never confused.
#[allow(clippy::too_many_arguments)]
fn picker(
    ui: &mut egui::Ui,
    id_salt: &str,
    editor: &mut PresetEditor,
    bank: &PresetBank,
    session: &PresetSession,
    params: &[&dyn Param],
    placeholder: &str,
    records: &[PresetRecord],
) -> PresetEvent {
    let mut event = PresetEvent::None;
    let current = session.current();
    let factory = || records.iter().filter(|r| r.preset.source == PresetSource::Factory);
    let user = || records.iter().filter(|r| r.preset.source == PresetSource::User);

    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(session.label(placeholder))
        .show_ui(ui, |ui| {
            if factory().next().is_some() {
                ui.label(egui::RichText::new("Factory").weak().small());
            }
            for record in factory() {
                if selectable(ui, &current, &record.preset) {
                    event = editor.pick(bank, session, &record.preset, params);
                }
            }
            if user().next().is_some() {
                ui.separator();
                ui.label(egui::RichText::new("User").weak().small());
                for record in user() {
                    if selectable(ui, &current, &record.preset) {
                        event = editor.pick(bank, session, &record.preset, params);
                    }
                }
            }
        });

    event
}

fn selectable(ui: &mut egui::Ui, current: &Option<PresetRef>, preset: &PresetRef) -> bool {
    let selected = current.as_ref().is_some_and(|c| c.matches(preset));
    ui.selectable_label(selected, preset.name.as_str()).clicked()
}

/// The inline name field shown while saving or renaming, in place of the
/// buttons.
fn name_entry_row(
    ui: &mut egui::Ui,
    editor: &mut PresetEditor,
    bank: &PresetBank,
    session: &PresetSession,
    params: &[&dyn Param],
) -> PresetEvent {
    ui.label(match editor.naming() {
        Some(NamingKind::Rename) => "Rename",
        _ => "Save as",
    });

    let mut submitted = false;
    if let Some(buf) = editor.name_buffer() {
        let response = ui.add(
            egui::TextEdit::singleline(buf)
                .desired_width(140.0)
                .hint_text("Preset name"),
        );
        response.request_focus();
        submitted = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    }
    submitted |= ui.small_button("OK").clicked();
    let cancelled =
        ui.small_button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));

    if submitted {
        editor.submit(bank, session, params)
    } else {
        if cancelled {
            editor.cancel();
        }
        PresetEvent::None
    }
}

// ---------------------------------------------------------------------------
// The browser overlay (§6.3)
// ---------------------------------------------------------------------------

/// Width at and above which the browser shows list and detail side by
/// side; narrower editors (gate, stereo) stack them.
pub const WIDE_LAYOUT: f32 = 720.0;

const DETAIL_WIDTH: f32 = 250.0;

/// The sorts the browser offers, as `(label, sort)`.
fn sort_options() -> [(&'static str, Sort); 5] {
    [
        ("Bank order", Sort::by(SortKey::Natural)),
        ("Name", Sort::by(SortKey::Title)),
        ("Category", Sort::by(SortKey::Field("category".into()))),
        ("Recently used", Sort::by(SortKey::RecentlyUsed)),
        (
            "Recently modified",
            Sort {
                key: SortKey::Field("modified".into()),
                descending: true,
            },
        ),
    ]
}

/// PUX-07: on Cocoa the dialog runs inside the guarded AppKit modal run
/// loop, so it stays a direct, synchronous `rfd` call there. Everywhere
/// else (the Wayland editor thread) it runs on [`FilePicker`]'s own
/// thread and is polled from `ctx.data`, so this file never blocks the
/// thread that would otherwise be pumping repaints and Wayland
/// dispatch while the native dialog is up.
#[cfg(target_os = "macos")]
fn pick_preset_file() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Resonance preset", &["json"])
        .pick_file()
}

#[cfg(target_os = "macos")]
fn save_preset_file(name: &str) -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Resonance preset", &["json"])
        .set_file_name(format!("{name}.json"))
        .save_file()
}

/// Non-macOS: the import/export pickers, kept alive across frames via
/// [`crate::file_picker::CtxPicker`] so they survive from the click
/// that opens the dialog to the frame the user answers it, without
/// needing a field on every plugin's editor `App` struct.
#[cfg(not(target_os = "macos"))]
mod picker {
    use crate::file_picker::{CtxPicker, FileDialogRequest, PickerAnswer};
    use plugin_gui_core::egui;

    /// Start `request`'s dialog for the picker keyed by `id`, unless
    /// one is already up for it. `with` is handed back unchanged by
    /// [`poll`] once the dialog resolves.
    pub(super) fn start<T: Clone + Send + Sync + 'static>(
        ctx: &egui::Context,
        id: egui::Id,
        request: FileDialogRequest,
        with: T,
    ) {
        CtxPicker::<T>::get(ctx, id).start(request, with);
    }

    /// The picker keyed by `id`'s answer and its `with` value, once
    /// the dialog has closed. Polling (not just starting) every frame
    /// is what keeps this entry alive in `ctx.data`'s temp storage.
    pub(super) fn poll<T: Clone + Send + Sync + 'static>(
        ctx: &egui::Context,
        id: egui::Id,
    ) -> Option<(Option<std::path::PathBuf>, T)> {
        let (answer, with) = CtxPicker::<T>::get(ctx, id).poll()?;
        Some((answer_into_one(answer), with))
    }

    fn answer_into_one(answer: PickerAnswer) -> Option<std::path::PathBuf> {
        answer.into_one()
    }
}

#[cfg(not(target_os = "macos"))]
fn import_picker_id() -> egui::Id {
    egui::Id::new("resonance_preset_import_picker")
}

#[cfg(not(target_os = "macos"))]
fn export_picker_id() -> egui::Id {
    egui::Id::new("resonance_preset_export_picker")
}

/// The browser: an `egui::Area` over the editor body, the plugin's
/// controls live underneath so an audition is audible. Keys: ↑/↓
/// audition, Enter keeps, Esc reverts and closes; a click outside keeps
/// and closes; × reverts and closes.
fn browser_overlay(
    ctx: &egui::Context,
    id_salt: &str,
    editor: &mut PresetEditor,
    bank: &PresetBank,
    session: &PresetSession,
    params: &[&dyn Param],
) -> PresetEvent {
    let mut event = PresetEvent::None;
    let screen = ctx.content_rect();
    let rect = screen.shrink(8.0);
    let wide = rect.width() >= WIDE_LAYOUT;
    let browser = &mut editor.browser;
    // Once per frame: the refresh (cheap unless the library moved) and the
    // one copy of the rows the widgets below read while `browser` is
    // borrowed mutably.
    browser.refresh(bank, crate::library_marks::BROWSER_POLL_INTERVAL);
    let rows = browser.rows().clone();
    let current_key = session
        .current()
        .filter(|c| c.is_resolved())
        .map(|c| crate::presets::mark_key(bank.plugin_id(), &c.id));

    let mut close: Option<bool> = None;
    let area = egui::Area::new(egui::Id::new((id_salt, "preset_browser")))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(theme::BG_1)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(6.0)
                .inner_margin(8.0)
                .show(ui, |ui| {
                    let inner = rect.size() - egui::vec2(16.0, 16.0);
                    ui.set_min_size(inner);
                    ui.set_max_size(inner);

                    // Header.
                    ui.horizontal(|ui| {
                        let title = egui::RichText::new("Presets").strong();
                        ui.label(title.color(theme::TEXT_1));
                        if let Some(name) = &bank.plugin_info().name {
                            let text = egui::RichText::new(format!("· {name}"));
                            ui.label(text.color(theme::TEXT_3));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let x = ui
                                .small_button("×")
                                .on_hover_text("Close (reverts an audition)");
                            if x.clicked() {
                                close = Some(false);
                            }
                        });
                    });

                    // Search, filters, sort.
                    ui.horizontal_wrapped(|ui| {
                        let width = if wide { 220.0 } else { 150.0 };
                        library_ui::search_field(ui, &mut browser.model, "Search presets", width);
                        // A combo does not wrap by itself: start a new row
                        // when the next one would not fit (a narrow editor).
                        let wrap = |ui: &mut egui::Ui| {
                            let need = ui.spacing().combo_width + ui.spacing().item_spacing.x;
                            if ui.available_size_before_wrap().x < need {
                                ui.end_row();
                            }
                        };
                        for (facet, label) in crate::presets::rows::FACETS {
                            wrap(ui);
                            library_ui::facet_menu(
                                ui,
                                (id_salt, "facet", facet),
                                label,
                                &mut browser.model,
                                &rows,
                                facet,
                            );
                        }
                        wrap(ui);
                        let mut fav = browser.model.favorites_only();
                        if ui.toggle_value(&mut fav, "★ only").changed() {
                            browser.model.set_favorites_only(fav);
                        }
                        let current_sort = browser.model.sort().clone();
                        let label = sort_options()
                            .into_iter()
                            .find(|(_, s)| *s == current_sort)
                            .map(|(l, _)| l)
                            .unwrap_or("Sort");
                        wrap(ui);
                        egui::ComboBox::from_id_salt((id_salt, "sort"))
                            .selected_text(label)
                            .show_ui(ui, |ui| {
                                for (l, s) in sort_options() {
                                    if ui.selectable_label(s == current_sort, l).clicked() {
                                        browser.model.set_sort(s);
                                    }
                                }
                            });
                    });
                    ui.label(
                        egui::RichText::new(format!("{} presets", browser.model.view_len()))
                            .size(11.0)
                            .color(theme::TEXT_3),
                    );

                    let footer_h = 26.0;
                    let body_h = (ui.available_height() - footer_h).max(60.0);
                    let columns = [ColumnSpec::left(70.0), ColumnSpec::right(14.0)];
                    // The list's keys (↑/↓ audition, Enter keep, Esc close)
                    // belong to whatever is in front: not while the
                    // metadata form or a rename has them.
                    let opts = ListOptions {
                        columns: &columns,
                        loaded: current_key.as_deref(),
                        keyboard: browser.form.is_none() && browser.rename.is_none(),
                        ..ListOptions::default()
                    };
                    let list = |ui: &mut egui::Ui, b: &mut crate::presets::PresetBrowser| {
                        library_ui::library_list(ui, (id_salt, "list"), &mut b.model, &rows, &opts)
                    };
                    let response = if wide {
                        let mut response = Default::default();
                        ui.horizontal_top(|ui| {
                            let list_w = (ui.available_width() - DETAIL_WIDTH - 8.0).max(120.0);
                            let down = egui::Layout::top_down(egui::Align::Min);
                            ui.allocate_ui_with_layout(egui::vec2(list_w, body_h), down, |ui| {
                                ui.set_min_height(body_h);
                                response = list(ui, browser);
                            });
                            ui.separator();
                            let detail = egui::vec2(DETAIL_WIDTH, body_h);
                            ui.allocate_ui_with_layout(detail, down, |ui| {
                                egui::ScrollArea::vertical()
                                    .id_salt((id_salt, "detail"))
                                    .show(ui, |ui| {
                                        let e = detail_pane(ui, id_salt, browser, bank, session);
                                        if !matches!(e, PresetEvent::None) {
                                            event = e;
                                        }
                                    });
                            });
                        });
                        response
                    } else {
                        let list_h = (body_h * 0.6).max(60.0);
                        let mut response = Default::default();
                        ui.allocate_ui(egui::vec2(ui.available_width(), list_h), |ui| {
                            ui.set_min_height(list_h);
                            response = list(ui, browser);
                        });
                        ui.separator();
                        egui::ScrollArea::vertical()
                            .id_salt((id_salt, "detail"))
                            .max_height((body_h - list_h - 8.0).max(40.0))
                            .show(ui, |ui| {
                                let e = detail_pane(ui, id_salt, browser, bank, session);
                                if !matches!(e, PresetEvent::None) {
                                    event = e;
                                }
                            });
                        response
                    };

                    if let Some(row) = response.star_clicked {
                        let key = rows.rows[row].key.clone();
                        browser.toggle_favorite(bank, &key);
                    }
                    if let Some(row) = response.clicked.or(response.moved) {
                        let key = rows.rows[row].key.clone();
                        let e = browser.audition(bank, session, params, &key);
                        if !matches!(e, PresetEvent::None) {
                            event = e;
                        }
                    }
                    if let Some(row) = response.double_clicked {
                        let key = rows.rows[row].key.clone();
                        let e = browser.commit(bank, session, params, Some(&key));
                        if !matches!(e, PresetEvent::None) {
                            event = e;
                        }
                    }
                    if response.escaped {
                        close = Some(false);
                    }

                    // Footer: the buttons first (right to left), then the
                    // notice and the hint in what is left, truncated — so
                    // the row never grows past a narrow editor.
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Save as…").clicked() {
                                browser.begin_save_as(bank, session);
                            }
                            #[cfg(target_os = "macos")]
                            if ui.small_button("Import…").clicked() {
                                if let Some(path) = pick_preset_file() {
                                    let e = browser.import(bank, params, &path);
                                    if !matches!(e, PresetEvent::None) {
                                        event = e;
                                    }
                                }
                            }
                            #[cfg(not(target_os = "macos"))]
                            {
                                if ui.small_button("Import…").clicked() {
                                    picker::start(
                                        ui.ctx(),
                                        import_picker_id(),
                                        crate::file_picker::FileDialogRequest::open_file()
                                            .filter("Resonance preset", &["json"]),
                                        (),
                                    );
                                }
                                if let Some((Some(path), ())) =
                                    picker::poll::<()>(ui.ctx(), import_picker_id())
                                {
                                    let e = browser.import(bank, params, &path);
                                    if !matches!(e, PresetEvent::None) {
                                        event = e;
                                    }
                                }
                            }
                            if let Some(n) = browser.model.notice() {
                                let color = if n.is_error() { theme::BAD } else { theme::TEXT_2 };
                                let text = egui::RichText::new(n.text()).size(11.0).color(color);
                                ui.add(egui::Label::new(text).truncate());
                            }
                            let hint = egui::RichText::new("↑↓ audition · Enter keep · Esc revert")
                                .size(11.0)
                                .color(theme::TEXT_3);
                            ui.add(egui::Label::new(hint).truncate());
                        });
                    });
                });
        });

    let outside = area.response.clicked_elsewhere()
        && !editor.browser_just_opened
        && editor.browser.form.is_none();
    if outside && close.is_none() {
        close = Some(true);
    }
    if let Some(keep) = close {
        let e = editor.browser.close(bank, session, params, keep);
        if !matches!(e, PresetEvent::None) {
            event = e;
        }
    }
    event
}

/// Pills for a facet's values (display only).
fn pills(ui: &mut egui::Ui, label: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    ui.label(egui::RichText::new(label).size(11.0).color(theme::TEXT_3));
    ui.horizontal_wrapped(|ui| {
        for v in values {
            tag_pill(ui, v, false, false);
        }
    });
}

/// The selected preset's detail: identity, metadata, personal tags and
/// the actions (§6.3 right-hand pane).
fn detail_pane(
    ui: &mut egui::Ui,
    id_salt: &str,
    browser: &mut crate::presets::PresetBrowser,
    bank: &PresetBank,
    session: &PresetSession,
) -> PresetEvent {
    let mut event = PresetEvent::None;
    let Some(key) = browser.model.selected().map(str::to_string) else {
        ui.label(egui::RichText::new("Select a preset").color(theme::TEXT_3));
        return event;
    };
    let Some(record) = browser.record(&key).cloned() else {
        return event;
    };
    let meta = &record.meta;
    let is_user = record.preset.source == PresetSource::User;

    match &mut browser.rename {
        Some((k, name)) if *k == key => {
            let resp = ui.add(egui::TextEdit::singleline(name).desired_width(200.0));
            resp.request_focus();
            let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                if submit || ui.small_button("Rename").clicked() {
                    event = browser.submit_rename(bank, session);
                }
                if ui.small_button("Cancel").clicked()
                    || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                {
                    browser.rename = None;
                }
            });
        }
        _ => {
            ui.label(egui::RichText::new(&meta.name).strong().color(theme::TEXT_1));
        }
    }
    let mut who = Vec::new();
    if let Some(a) = &meta.author {
        who.push(format!("by {a}"));
    }
    who.push(record.preset.source.as_str().to_string());
    ui.label(egui::RichText::new(who.join(" · ")).size(11.0).color(theme::TEXT_2));
    // The actions sit right under the name, so the narrowest editor still
    // reaches them without scrolling past the metadata.
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        let edit_label = if is_user { "Edit info…" } else { "My tags…" };
        if ui.small_button(edit_label).clicked() {
            browser.begin_edit(bank, &key);
        }
        if ui.small_button("Duplicate").clicked() {
            event = browser.duplicate(bank, &key);
        }
        if ui.add_enabled(is_user, egui::Button::new("Rename").small()).clicked() {
            browser.begin_rename(&key);
        }
        #[cfg(target_os = "macos")]
        if ui.small_button("Export…").clicked() {
            if let Some(path) = save_preset_file(&meta.name) {
                browser.export(bank, &key, &path);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if ui.small_button("Export…").clicked() {
                picker::start(
                    ui.ctx(),
                    export_picker_id(),
                    crate::file_picker::FileDialogRequest::save_file()
                        .filter("Resonance preset", &["json"])
                        .file_name(format!("{}.json", meta.name)),
                    key.clone(),
                );
            }
            if let Some((Some(path), exported_key)) =
                picker::poll::<String>(ui.ctx(), export_picker_id())
            {
                browser.export(bank, &exported_key, &path);
            }
        }
        if let (true, Some(path)) = (is_user, &record.path) {
            if ui.small_button("Reveal").clicked() {
                if let Err(e) = crate::reveal::reveal(path) {
                    browser.model.set_error(format!("Could not show the file: {e}"));
                }
            }
        }
        if ui.add_enabled(is_user, egui::Button::new("Delete").small()).clicked() {
            browser.model.begin_delete(key.clone());
        }
    });
    let prompt = format!("Delete '{}'?", meta.name);
    if let ConfirmOutcome::Confirmed(k) = library_ui::confirm_delete_row(
        ui,
        &mut browser.model,
        &key,
        &prompt,
        Some("It moves to the trash and can be recovered for 30 days."),
    ) {
        event = browser.delete(bank, session, &k);
    }
    ui.add_space(4.0);
    if let Some(v) = &record.plugin_version {
        let text = egui::RichText::new(format!("saved with {v}"));
        ui.label(text.size(11.0).color(theme::TEXT_3));
    }
    if let Some(from) = &meta.derived_from {
        let base = browser
            .rows()
            .rows
            .iter()
            .find(|r| r.record.preset.id == *from)
            .map(|r| r.record.meta.name.clone())
            .unwrap_or_else(|| from.clone());
        let text = egui::RichText::new(format!("based on {base}"));
        ui.label(text.size(11.0).color(theme::TEXT_3));
    }
    if let Some(d) = &meta.description {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(d).size(11.5).color(theme::TEXT_2));
    }
    ui.add_space(4.0);
    pills(ui, "category", &meta.category.iter().cloned().collect::<Vec<_>>());
    pills(ui, "for", &meta.instrument);
    pills(ui, "genres", &meta.genres);
    pills(ui, "character", &meta.character);
    pills(ui, "tags", &meta.tags);

    // Personal tags (every preset, factory ones included).
    ui.label(egui::RichText::new("my tags").size(11.0).color(theme::TEXT_3));
    let personal = bank
        .library()
        .preset_marks(bank.plugin_id(), &record.preset.id)
        .tags;
    // Completion is recomputed when the draft (or the tags) change, not
    // every frame.
    let wanted = format!("{}\u{1f}{}", browser.tag_draft, personal.join(","));
    if browser.tag_suggest.as_ref().map(|(k, _)| k) != Some(&wanted) {
        let list = if browser.tag_draft.trim().is_empty() {
            Vec::new()
        } else {
            bank.library().marks().complete_tag(&browser.tag_draft, &personal, 6)
        };
        browser.tag_suggest = Some((wanted, list));
    }
    let suggestions = browser
        .tag_suggest
        .as_ref()
        .map(|(_, s)| s.clone())
        .unwrap_or_default();
    let tr = library_ui::tag_row(
        ui,
        (id_salt, "personal", &key),
        &personal,
        &mut browser.tag_draft,
        &suggestions,
    );
    if let Some(t) = tr.added {
        browser.edit_personal_tag(bank, &key, &t, true);
    }
    if let Some(t) = tr.removed {
        browser.edit_personal_tag(bank, &key, &t, false);
    }

    event
}

// ---------------------------------------------------------------------------
// The metadata form (§6.5)
// ---------------------------------------------------------------------------

/// Tag completion for one facet row: the seeded values, then used ones.
fn facet_suggestions(
    bank: &PresetBank,
    facet: &str,
    draft: &str,
    exclude: &[String],
) -> Vec<String> {
    use crate::library_marks::vocab::Facet;
    let draft = crate::library_marks::normalize_tag(draft).unwrap_or_default();
    if draft.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<String> = Facet::from_name(facet)
        .map(|f| f.seeded().iter().map(|s| s.to_string()).collect())
        .unwrap_or_default();
    out.extend(bank.library().marks().complete_tag(&draft, exclude, 12));
    let mut seen = Vec::new();
    out.retain(|v| {
        let keep = v.starts_with(&draft) && !exclude.contains(v) && !seen.contains(v);
        seen.push(v.clone());
        keep
    });
    out.truncate(6);
    out
}

fn tag_field(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    bank: &PresetBank,
    label: &str,
    facet: &str,
    values: &mut Vec<String>,
    draft: &mut String,
) {
    ui.horizontal(|ui| {
        let text = egui::RichText::new(label).color(theme::TEXT_2);
        ui.add_sized([72.0, 18.0], egui::Label::new(text));
        let suggestions = facet_suggestions(bank, facet, draft, values);
        let r = library_ui::tag_row(ui, id, values, draft, &suggestions);
        if let Some(t) = r.added.and_then(|t| crate::library_marks::normalize_tag(&t)) {
            if !values.contains(&t) {
                values.push(t);
            }
        }
        if let Some(t) = r.removed {
            values.retain(|v| *v != t);
        }
    });
}

/// The metadata form: Save as… (a new preset), Edit info… (a user
/// preset's own metadata) or, on a factory preset, marks only.
fn form_overlay(
    ctx: &egui::Context,
    id_salt: &str,
    editor: &mut PresetEditor,
    bank: &PresetBank,
    session: &PresetSession,
    params: &[&dyn Param],
) -> PresetEvent {
    let mut event = PresetEvent::None;
    let screen = ctx.content_rect();
    let width = (screen.width() - 32.0).clamp(240.0, 480.0);
    let pos = egui::pos2(screen.center().x - width / 2.0, screen.min.y + 16.0);
    let mut submit: Option<bool> = None;
    let mut cancel = false;
    let Some(form) = editor.browser.form.as_mut() else {
        return event;
    };
    egui::Area::new(egui::Id::new((id_salt, "preset_form")))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(theme::BG_2)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(6.0)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.set_width(width - 20.0);
                    let title = match form.mode {
                        FormMode::SaveAs => "Save preset",
                        FormMode::EditInfo(_) => "Edit preset info",
                        FormMode::MarksOnly(_) => "My marks",
                    };
                    ui.label(egui::RichText::new(title).strong().color(theme::TEXT_1));
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height((screen.height() - 120.0).max(80.0))
                        .show(ui, |ui| {
                            form_fields(ui, id_salt, form, bank);
                        });
                    if form.name_clash {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "⚠ A user preset named \"{}\" exists.",
                                    form.name.trim()
                                ))
                                .color(theme::WARM),
                            );
                            if ui.small_button("Overwrite").clicked() {
                                submit = Some(true);
                            }
                        });
                    }
                    if let Some(e) = &form.error {
                        ui.label(egui::RichText::new(e).color(theme::BAD));
                    }
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Save").clicked() {
                                submit = Some(false);
                            }
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        });
                    });
                    // The form takes Esc for itself: it closes the form, not
                    // the browser behind it.
                    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                        cancel = true;
                    }
                });
        });
    if cancel {
        editor.browser.form = None;
    } else if let Some(overwrite) = submit {
        event = editor.browser.submit_form(bank, session, params, overwrite);
    }
    event
}

fn form_fields(ui: &mut egui::Ui, id_salt: &str, form: &mut MetaForm, bank: &PresetBank) {
    use crate::library_marks::vocab;
    let row = |ui: &mut egui::Ui, label: &str| {
        let text = egui::RichText::new(label).color(theme::TEXT_2);
        ui.add_sized([72.0, 18.0], egui::Label::new(text));
    };
    if matches!(form.mode, FormMode::MarksOnly(_)) {
        ui.horizontal(|ui| {
            row(ui, "Favourite");
            if star_toggle(ui, form.favorite).clicked() {
                form.favorite = !form.favorite;
            }
        });
        let [_, _, _, draft] = &mut form.drafts;
        let tags = &mut form.personal_tags;
        tag_field(ui, (id_salt, "f-tags"), bank, "My tags", TAGS_FACET, tags, draft);
        return;
    }
    ui.horizontal(|ui| {
        row(ui, "Name");
        ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(f32::INFINITY));
    });
    form.meta.name = form.name.clone();
    ui.horizontal(|ui| {
        row(ui, "Author");
        let mut author = form.meta.author.clone().unwrap_or_default();
        if ui
            .add(egui::TextEdit::singleline(&mut author).desired_width(f32::INFINITY))
            .changed()
        {
            form.meta.author = Some(author);
        }
    });
    ui.horizontal(|ui| {
        row(ui, "Category");
        let current = form.meta.category.clone().unwrap_or_default();
        egui::ComboBox::from_id_salt((id_salt, "f-category"))
            .selected_text(if current.is_empty() { "(none)" } else { &current })
            .show_ui(ui, |ui| {
                if ui.selectable_label(current.is_empty(), "(none)").clicked() {
                    form.meta.category = None;
                }
                for c in vocab::CATEGORIES_INSTRUMENT.iter().chain(vocab::CATEGORIES_EFFECT) {
                    if ui.selectable_label(current == *c, *c).clicked() {
                        form.meta.category = Some(c.to_string());
                    }
                }
            });
    });
    let [d0, d1, d2, d3] = &mut form.drafts;
    let m = &mut form.meta;
    tag_field(ui, (id_salt, "f-for"), bank, "For", "instrument", &mut m.instrument, d0);
    tag_field(ui, (id_salt, "f-genres"), bank, "Genres", "genres", &mut m.genres, d1);
    tag_field(ui, (id_salt, "f-char"), bank, "Character", "character", &mut m.character, d2);
    tag_field(ui, (id_salt, "f-tags"), bank, "Tags", TAGS_FACET, &mut m.tags, d3);
    ui.horizontal(|ui| {
        row(ui, "Description");
        let mut desc = form.meta.description.clone().unwrap_or_default();
        if ui
            .add(
                egui::TextEdit::multiline(&mut desc)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            )
            .changed()
        {
            form.meta.description = Some(desc);
        }
    });
}
