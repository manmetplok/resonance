//! Tone3000 browser overlay panel.
//!
//! Rendered on top of the normal amp editor when the user clicks the
//! "Tone3000…" button in the header. All network activity is delegated
//! to [`crate::tone3000::worker`], so this file is purely presentation:
//! it reads the shared `State` each frame, lays out egui widgets, and
//! posts `Command`s back.
//!
//! Layout:
//! ```text
//! ┌───────────────────────────────────────────────────┐
//! │ TONE3000            [status pill]    [Close] [X] │
//! │ ──────────────────────────────────────────────── │
//! │ [search query_________________]  [Search]        │
//! │ ──────────────────────────────────────────────── │
//! │ Tones (left)        │  Models (right)            │
//! │ ─────────────────── │  ─────────────────────     │
//! │ • title — author    │  • name (size)  [Download] │
//! │ • title — author    │  • name (size)  [Download] │
//! │ …                   │  …                         │
//! └───────────────────────────────────────────────────┘
//! ```

use std::sync::Arc;

use plugin_gui_core::egui;

use super::theme;
use crate::tone3000::client::ArchitectureFilter;
use crate::tone3000::worker::{Command, Status, WorkerHandle};

/// Sort modes surfaced in the UI dropdown. The string values match the
/// exact query-string tokens tone3000.com's search API accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Trending,
    Downloads,
    Newest,
    Oldest,
    BestMatch,
}

impl SortMode {
    pub const ALL: &'static [SortMode] = &[
        SortMode::Trending,
        SortMode::Downloads,
        SortMode::Newest,
        SortMode::Oldest,
        SortMode::BestMatch,
    ];

    pub fn api_value(&self) -> &'static str {
        match self {
            SortMode::Trending => "trending",
            SortMode::Downloads => "downloads-all-time",
            SortMode::Newest => "newest",
            SortMode::Oldest => "oldest",
            SortMode::BestMatch => "best-match",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SortMode::Trending => "Trending",
            SortMode::Downloads => "Most downloads",
            SortMode::Newest => "Newest",
            SortMode::Oldest => "Oldest",
            SortMode::BestMatch => "Best match",
        }
    }
}

pub struct Tone3000PanelState {
    pub open: bool,
    pub query: String,
    pub sort: SortMode,
    /// Which NAM architectures to browse. Defaults to
    /// [`ArchitectureFilter::All`] — the engine plays A1 and A2 alike,
    /// so the browser shows the whole catalogue unless the user narrows
    /// it deliberately.
    pub architecture: ArchitectureFilter,
    /// Set once after the user connects so we auto-populate the tone
    /// list with the default popularity sort instead of showing an
    /// empty panel.
    pub did_initial_fetch: bool,
}

impl Default for Tone3000PanelState {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            sort: SortMode::Trending,
            architecture: ArchitectureFilter::All,
            did_initial_fetch: false,
        }
    }
}

impl Tone3000PanelState {
    /// The search command this panel's current filters describe.
    fn search_command(&self) -> Command {
        Command::Search {
            query: self.query.clone(),
            sort: self.sort.api_value().to_string(),
            architecture: self.architecture,
        }
    }
}

pub fn draw(ui: &mut egui::Ui, panel: &mut Tone3000PanelState, worker: &Arc<WorkerHandle>) {
    // Dim the underlying editor behind the overlay.
    let screen = ui.ctx().content_rect();
    ui.painter()
        .rect_filled(screen, 0.0, egui::Color32::from_black_alpha(180));

    let margin = 32.0;
    let rect = screen.shrink(margin);
    let window_id = egui::Id::new("tone3000_panel_window");

    egui::Area::new(window_id)
        .fixed_pos(rect.min)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            let frame = egui::Frame::new()
                .fill(theme::PANEL)
                .stroke(egui::Stroke::new(1.0, theme::BORDER))
                .inner_margin(egui::Margin::same(14));
            frame.show(ui, |ui| {
                ui.set_width(rect.width());
                ui.set_height(rect.height());
                draw_contents(ui, panel, worker);
            });
        });
}

fn draw_contents(ui: &mut egui::Ui, panel: &mut Tone3000PanelState, worker: &Arc<WorkerHandle>) {
    draw_header(ui, panel, worker);
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);

    // Once the user is authenticated, kick off an initial trending
    // search so the panel isn't empty when they open it.
    let connected = matches!(worker.state.lock().status, Status::Connected);
    if connected && !panel.did_initial_fetch {
        panel.did_initial_fetch = true;
        worker.send(panel.search_command());
    }

    draw_search_row(ui, panel, worker);
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);

    // One lock, one consistent snapshot for the whole frame.
    let snapshot = {
        let s = worker.state.lock();
        Snapshot {
            status: s.status.clone(),
            tones: s.tones.clone(),
            models: s.models.clone(),
            selected_tone: s.selected_tone,
            error: s.last_error.clone(),
            total_tones: s.total_tones,
            has_more: s.has_more,
        }
    };

    draw_results(ui, worker, &snapshot);

    if let Some(err) = snapshot.error {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(err).color(theme::DANGER).size(11.0));
    }
}

fn draw_header(ui: &mut egui::Ui, panel: &mut Tone3000PanelState, worker: &Arc<WorkerHandle>) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("TONE3000")
                .strong()
                .color(theme::ACCENT)
                .size(14.0),
        );
        ui.add_space(10.0);

        let status = worker.state.lock().status.clone();
        draw_status_pill(ui, &status);

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Close").clicked() {
                panel.open = false;
            }
            ui.add_space(6.0);
            match status {
                Status::Disconnected | Status::Error(_) => {
                    if ui.button("Connect…").clicked() {
                        worker.send(Command::Authenticate);
                    }
                }
                Status::Connected
                | Status::Searching
                | Status::LoadingModels
                | Status::Downloading(_) => {
                    if ui.button("Disconnect").clicked() {
                        worker.send(Command::Disconnect);
                        panel.did_initial_fetch = false;
                    }
                }
                Status::Authenticating => {
                    ui.add_enabled(false, egui::Button::new("Authenticating…"));
                }
            }
        });
    });
}

fn draw_status_pill(ui: &mut egui::Ui, status: &Status) {
    let (text, color): (String, egui::Color32) = match status {
        Status::Disconnected => ("disconnected".into(), theme::TEXT_DIM),
        Status::Authenticating => ("authenticating…".into(), theme::WARN),
        Status::Connected => ("connected".into(), theme::TUNE_OK),
        Status::Searching => ("searching…".into(), theme::ACCENT),
        Status::LoadingModels => ("loading models…".into(), theme::ACCENT),
        Status::Downloading(name) => (format!("downloading {name}…"), theme::ACCENT),
        Status::Error(e) => (format!("error: {e}"), theme::DANGER),
    };
    ui.label(egui::RichText::new(text).color(color).size(11.0));
}

fn draw_search_row(ui: &mut egui::Ui, panel: &mut Tone3000PanelState, worker: &Arc<WorkerHandle>) {
    let connected = matches!(
        worker.state.lock().status,
        Status::Connected | Status::Searching | Status::LoadingModels | Status::Downloading(_)
    );
    ui.horizontal(|ui| {
        ui.add_enabled_ui(connected, |ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut panel.query)
                    .hint_text("search amp tones…")
                    .desired_width(320.0),
            );
            let submitted = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

            let sort_changed = draw_sort_dropdown(ui, &mut panel.sort);
            let arch_changed = draw_architecture_dropdown(ui, &mut panel.architecture);

            if ui.button("Search").clicked() || submitted || sort_changed || arch_changed {
                worker.send(panel.search_command());
            }
        });
    });
}

/// Architecture filter dropdown. Replaces the hidden Architecture-2 pin
/// that used to hide most of the catalogue (ba todo #1315).
fn draw_architecture_dropdown(ui: &mut egui::Ui, arch: &mut ArchitectureFilter) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt("tone3000_arch_combo")
        .selected_text(arch.label())
        .show_ui(ui, |ui| {
            for &mode in ArchitectureFilter::ALL {
                if ui.selectable_label(*arch == mode, mode.label()).clicked() {
                    *arch = mode;
                    changed = true;
                }
            }
        });
    changed
}

fn draw_sort_dropdown(ui: &mut egui::Ui, sort: &mut SortMode) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt("tone3000_sort_combo")
        .selected_text(sort.label())
        .show_ui(ui, |ui| {
            for &mode in SortMode::ALL {
                if ui.selectable_label(*sort == mode, mode.label()).clicked() {
                    *sort = mode;
                    changed = true;
                }
            }
        });
    changed
}

/// Everything the results area renders, copied out of the worker state
/// under one lock so a frame can't tear across two reads.
struct Snapshot {
    status: Status,
    tones: Vec<crate::tone3000::types::Tone>,
    models: Vec<crate::tone3000::types::Model>,
    selected_tone: Option<i64>,
    error: Option<String>,
    total_tones: Option<u32>,
    has_more: bool,
}

fn draw_results(ui: &mut egui::Ui, worker: &Arc<WorkerHandle>, snap: &Snapshot) {
    let Snapshot {
        status,
        tones,
        models,
        selected_tone,
        total_tones,
        has_more,
        ..
    } = snap;
    let (selected_tone, total_tones, has_more) = (*selected_tone, *total_tones, *has_more);

    let avail = ui.available_size_before_wrap();
    let left_w = (avail.x * 0.55).max(280.0).min(avail.x - 280.0);

    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(left_w, avail.y - 30.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.label(
                    egui::RichText::new(tones_heading(tones.len(), total_tones))
                        .color(theme::TEXT_DIM)
                        .size(11.0),
                );
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("tone3000_tones_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for tone in tones {
                            draw_tone_row(ui, worker, tone, selected_tone);
                        }
                        if tones.is_empty() && matches!(status, Status::Connected) {
                            ui.label(
                                egui::RichText::new("(no results — try a search)")
                                    .color(theme::TEXT_DIM)
                                    .size(11.0),
                            );
                        }
                        if has_more {
                            ui.add_space(4.0);
                            ui.vertical_centered(|ui| {
                                ui.add_enabled_ui(!status.is_busy(), |ui| {
                                    if ui.button("Load more").clicked() {
                                        worker.send(Command::LoadMore);
                                    }
                                });
                            });
                            ui.add_space(4.0);
                        }
                    });
            },
        );

        ui.separator();

        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), avail.y - 30.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.label(
                    egui::RichText::new(format!("Models ({})", models.len()))
                        .color(theme::TEXT_DIM)
                        .size(11.0),
                );
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("tone3000_models_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if selected_tone.is_none() {
                            ui.label(
                                egui::RichText::new("(pick a tone on the left)")
                                    .color(theme::TEXT_DIM)
                                    .size(11.0),
                            );
                            return;
                        }
                        for model in models {
                            draw_model_row(ui, worker, model);
                        }
                        if models.is_empty() {
                            ui.label(
                                egui::RichText::new("(no models on this tone)")
                                    .color(theme::TEXT_DIM)
                                    .size(11.0),
                            );
                        }
                    });
            },
        );
    });
}

/// Heading over the tone list. Shows how many of the server's total are
/// on screen, so 25 rows no longer read as "that is the whole catalogue".
pub fn tones_heading(shown: usize, total: Option<u32>) -> String {
    match total {
        Some(total) if (total as usize) > shown => format!("Tones ({shown} of {total})"),
        Some(total) => format!("Tones ({total})"),
        None => format!("Tones ({shown})"),
    }
}

fn draw_tone_row(
    ui: &mut egui::Ui,
    worker: &Arc<WorkerHandle>,
    tone: &crate::tone3000::types::Tone,
    selected_tone: Option<i64>,
) {
    let selected = selected_tone == Some(tone.id);
    let bg = if selected {
        theme::PANEL_LIGHT
    } else {
        theme::PANEL
    };

    let frame = egui::Frame::new()
        .fill(bg)
        .stroke(egui::Stroke::new(
            1.0,
            if selected {
                theme::ACCENT
            } else {
                theme::BORDER
            },
        ))
        .inner_margin(egui::Margin::same(6))
        .outer_margin(egui::Margin::symmetric(0, 2));

    // Render the row inside a Frame and capture its full outer rect,
    // then promote the whole rect to a click target so clicking
    // anywhere on the row — not just the text — selects the tone.
    let frame_resp = frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(tone.display_title())
                    .color(theme::TEXT)
                    .size(12.0)
                    .strong(),
            );
            let subtitle = format!(
                "by {} · {} models · {} downloads",
                tone.display_author(),
                tone.models_count.unwrap_or(0),
                tone.downloads_count.unwrap_or(0),
            );
            ui.label(
                egui::RichText::new(subtitle)
                    .color(theme::TEXT_DIM)
                    .size(10.0),
            );
        });
    });

    let click_id = ui.id().with(("tone3000_tone_row", tone.id));
    let click = ui.interact(frame_resp.response.rect, click_id, egui::Sense::click());
    if click.clicked() {
        worker.send(Command::ListModels(tone.id));
    }
}

fn draw_model_row(
    ui: &mut egui::Ui,
    worker: &Arc<WorkerHandle>,
    model: &crate::tone3000::types::Model,
) {
    let frame = egui::Frame::new()
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .inner_margin(egui::Margin::same(6))
        .outer_margin(egui::Margin::symmetric(0, 2));

    frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(model.display_label())
                    .color(theme::TEXT)
                    .size(12.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enabled = model.model_url.is_some();
                ui.add_enabled_ui(enabled, |ui| {
                    if ui.button("Download").clicked() {
                        worker.send(Command::Download(model.clone()));
                    }
                });
            });
        });
    });
}
