//! Wavetable plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Layout: a top tab bar switches between five tabs — OSC, ENV/FLT, LFO,
//! MOD, FX — each of which renders its own controls and canvas-based
//! visualisations. The `WavetableEditorFactory` implements
//! `resonance_plugin::gui::EditorFactory` and is returned from the plugin's
//! `editor_factory()` hook.

use std::sync::Arc;

use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{egui, Editor as RuntimeEditor, EditorApp, EditorOptions};

use crate::params::{WavetableParams, PARAM_COUNT};
use crate::presets::PRESETS;
use crate::viz::{VizSnapshot, WavetableVizState};

mod display_waves;
mod tabs;
mod theme;
mod viz;
mod widgets;

// ---------------------------------------------------------------------------
// Factory — produced by ResonanceWavetable::editor_factory().
// ---------------------------------------------------------------------------

pub struct WavetableEditorFactory {
    params: Arc<WavetableParams>,
    viz: Arc<WavetableVizState>,
}

impl WavetableEditorFactory {
    pub fn new(params: Arc<WavetableParams>, viz: Arc<WavetableVizState>) -> Self {
        Self { params, viz }
    }
}

impl EditorFactory for WavetableEditorFactory {
    fn supports(&self, api_name: &str, is_floating: bool) -> bool {
        is_floating && api_name == "wayland"
    }

    fn preferred(&self) -> Option<(&'static str, bool)> {
        Some(("wayland", true))
    }

    fn preferred_size(&self) -> (u32, u32) {
        (960, 560)
    }

    fn create(&self, api_name: &str, is_floating: bool) -> Option<Box<dyn PluginEditor>> {
        if !self.supports(api_name, is_floating) {
            return None;
        }
        let app = WavetableEditorApp::new(self.params.clone(), self.viz.clone());
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Wavetable".to_string(),
                initial_size: (960, 560),
                min_size: (720, 480),
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle {
            runtime: Some(runtime),
            size: (960, 560),
        }))
    }
}

// ---------------------------------------------------------------------------
// RuntimeEditorHandle — bridges `PluginEditor` to `wayland_plugin_gui::Editor`.
// ---------------------------------------------------------------------------

struct RuntimeEditorHandle {
    runtime: Option<RuntimeEditor>,
    size: (u32, u32),
}

impl PluginEditor for RuntimeEditorHandle {
    fn show(&mut self) {
        if let Some(r) = &self.runtime {
            r.show();
        }
    }

    fn hide(&mut self) {
        if let Some(r) = &self.runtime {
            r.hide();
        }
    }

    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn set_size(&mut self, width: u32, height: u32) -> bool {
        if let Some(r) = &self.runtime {
            if r.set_size(width, height).is_ok() {
                self.size = (width, height);
                return true;
            }
        }
        false
    }

    fn can_resize(&self) -> bool {
        self.runtime
            .as_ref()
            .map(|r| r.is_resizable())
            .unwrap_or(false)
    }

    fn set_title(&mut self, _title: &str) {
        // Not wired into the runtime yet; the plan is to forward this via a
        // new Command variant. Left as a follow-up — the DAW doesn't call
        // suggest_title right now anyway.
    }
}

impl Drop for RuntimeEditorHandle {
    fn drop(&mut self) {
        if let Some(r) = self.runtime.take() {
            r.destroy();
        }
    }
}

// ---------------------------------------------------------------------------
// EditorApp — the actual egui UI that runs on the editor thread.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WtTab {
    Osc,
    EnvFilter,
    Lfo,
    Mod,
    Fx,
}

pub(crate) struct WavetableEditorApp {
    pub(crate) params: Arc<WavetableParams>,
    pub(crate) viz: Arc<WavetableVizState>,
    pub(crate) selected_tab: WtTab,
    pub(crate) selected_osc: usize,
    pub(crate) selected_lfo: usize,
    #[allow(dead_code)] // reserved for future "highlight selected mod slot" feature
    pub(crate) selected_mod_slot: usize,
    pub(crate) preset_idx: usize,
    /// Most recent audio→UI viz snapshot, refreshed each frame.
    pub(crate) snapshot: VizSnapshot,
}

impl WavetableEditorApp {
    pub fn new(params: Arc<WavetableParams>, viz: Arc<WavetableVizState>) -> Self {
        let snapshot = viz.read_snapshot();
        // Standalone editor harness can pick an initial tab via env var so
        // each tab can be screenshotted without manual clicking.
        let selected_tab = match std::env::var("WT_TAB").as_deref() {
            Ok("env") => WtTab::EnvFilter,
            Ok("lfo") => WtTab::Lfo,
            Ok("mod") => WtTab::Mod,
            Ok("fx") => WtTab::Fx,
            _ => WtTab::Osc,
        };
        Self {
            params,
            viz,
            selected_tab,
            selected_osc: 0,
            selected_lfo: 0,
            selected_mod_slot: 0,
            preset_idx: 0,
            snapshot,
        }
    }
}

impl EditorApp for WavetableEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        // Refresh live audio state for this frame.
        self.snapshot = self.viz.read_snapshot();
        // Drive continuous ~60 Hz repaint so live views animate.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        theme::apply(ui.ctx());

        // Chrome (brand + chrome icons).
        egui::Panel::top("wt_chrome")
            .exact_size(38.0)
            .frame(
                egui::Frame::default()
                    .fill(theme::BG_1)
                    .inner_margin(egui::Margin::symmetric(14, 6))
                    .stroke(egui::Stroke::new(1.0, theme::LINE_2)),
            )
            .show_inside(ui, |ui| draw_chrome(ui, self));

        // Tab bar (module nav + preset pill + voices badge).
        egui::Panel::top("wt_tabs")
            .exact_size(48.0)
            .frame(
                egui::Frame::default()
                    .fill(theme::BG_1)
                    .inner_margin(egui::Margin::symmetric(14, 6))
                    .stroke(egui::Stroke::new(1.0, theme::LINE_2)),
            )
            .show_inside(ui, |ui| draw_tab_bar(ui, self));

        // Status bar.
        egui::Panel::bottom("wt_status")
            .exact_size(28.0)
            .frame(
                egui::Frame::default()
                    .fill(theme::BG_1)
                    .inner_margin(egui::Margin::symmetric(16, 6))
                    .stroke(egui::Stroke::new(1.0, theme::LINE_2)),
            )
            .show_inside(ui, |ui| draw_status_bar(ui, self));

        // Body.
        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(theme::BG_0)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show_inside(ui, |ui| match self.selected_tab {
                WtTab::Osc => tabs::osc::draw(ui, self),
                WtTab::EnvFilter => tabs::env_filter::draw(ui, self),
                WtTab::Lfo => tabs::lfo::draw(ui, self),
                WtTab::Mod => tabs::mod_matrix::draw(ui, self),
                WtTab::Fx => tabs::fx::draw(ui, self),
            });
    }
}

fn draw_chrome(ui: &mut egui::Ui, _app: &mut WavetableEditorApp) {
    ui.horizontal_centered(|ui| {
        // Traffic-light dots (decorative).
        let dot = |ui: &mut egui::Ui, color: egui::Color32| {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 5.0, color);
        };
        dot(ui, egui::Color32::from_rgb(0xed, 0x6b, 0x5e));
        ui.add_space(4.0);
        dot(ui, egui::Color32::from_rgb(0xf4, 0xbe, 0x4f));
        ui.add_space(4.0);
        dot(ui, egui::Color32::from_rgb(0x61, 0xc4, 0x54));

        ui.add_space(14.0);
        ui.label(egui::RichText::new("●").color(theme::ACCENT).size(11.0));
        ui.add_space(2.0);
        ui.label(egui::RichText::new("Resonance").color(theme::TEXT_2).size(12.0));
        ui.label(egui::RichText::new("/").color(theme::TEXT_4).size(12.0));
        ui.label(
            egui::RichText::new("Wavetable")
                .italics()
                .color(theme::TEXT_1)
                .size(15.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new("⚙").color(theme::TEXT_3).size(13.0));
            ui.add_space(10.0);
            ui.label(egui::RichText::new("A").color(theme::TEXT_3).size(12.0));
            ui.add_space(10.0);
            ui.label(egui::RichText::new("?").color(theme::TEXT_3).size(13.0));
        });
    });
}

fn draw_tab_bar(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.label(
            egui::RichText::new("WAVETABLE")
                .color(theme::TEXT_3)
                .size(10.5)
                .strong(),
        );
        ui.add_space(8.0);

        let labels = ["Osc", "Env · Filter", "LFO", "Mod", "FX"];
        let selected_idx = match app.selected_tab {
            WtTab::Osc => 0,
            WtTab::EnvFilter => 1,
            WtTab::Lfo => 2,
            WtTab::Mod => 3,
            WtTab::Fx => 4,
        };
        if let Some(i) = widgets::segmented(ui, &labels, selected_idx, false) {
            app.selected_tab = match i {
                0 => WtTab::Osc,
                1 => WtTab::EnvFilter,
                2 => WtTab::Lfo,
                3 => WtTab::Mod,
                _ => WtTab::Fx,
            };
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Voices badge — paint manually so the right-to-left parent
            // doesn't expand the frame to fill the remaining width.
            let voices = app.snapshot.active_voice_count;
            let max_voices = app.params.max_voices.value();
            let badge_text = format!("{} / {}", voices, max_voices);
            draw_voices_badge(ui, &badge_text);

            ui.add_space(8.0);

            // Preset pill — explicit left-to-right inner layout so arrows
            // stay in the natural order (◀ name ▶).
            let pill = egui::Frame::default()
                .fill(theme::BG_2)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(7.0)
                .inner_margin(egui::Margin::symmetric(10, 4));
            pill.show(ui, |ui| {
                ui.with_layout(
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("◀")
                                        .color(theme::TEXT_3)
                                        .size(9.0),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            app.preset_idx = app.preset_idx.saturating_sub(1);
                            if let Some(entry) = PRESETS.get(app.preset_idx) {
                                load_preset(&app.params, entry.json);
                            }
                        }
                        let preset_name = PRESETS
                            .get(app.preset_idx)
                            .map(|p| p.name)
                            .unwrap_or("— select —");
                        egui::ComboBox::from_id_salt("wt_preset_combo")
                            .width(170.0)
                            .selected_text(
                                egui::RichText::new(preset_name)
                                    .color(theme::TEXT_1)
                                    .size(12.0),
                            )
                            .show_ui(ui, |ui| {
                                for (i, entry) in PRESETS.iter().enumerate() {
                                    if ui.selectable_label(false, entry.name).clicked() {
                                        app.preset_idx = i;
                                        load_preset(&app.params, entry.json);
                                    }
                                }
                            });
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("▶")
                                        .color(theme::TEXT_3)
                                        .size(9.0),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            let next = app.preset_idx + 1;
                            if next < PRESETS.len() {
                                app.preset_idx = next;
                                if let Some(entry) = PRESETS.get(app.preset_idx) {
                                    load_preset(&app.params, entry.json);
                                }
                            }
                        }
                    },
                );
            });

            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("PRESET")
                    .color(theme::TEXT_3)
                    .size(10.0)
                    .strong(),
            );
        });
    });
}

fn draw_status_bar(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.horizontal_centered(|ui| {
        let mono = |ui: &mut egui::Ui, label: &str, value: &str| {
            ui.label(
                egui::RichText::new(label)
                    .color(theme::TEXT_3)
                    .size(10.0)
                    .strong(),
            );
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(value)
                    .color(theme::TEXT_2)
                    .size(10.5)
                    .monospace(),
            );
            ui.add_space(14.0);
        };

        mono(ui, "SR", "48000 Hz");
        mono(ui, "BUF", "256");
        let voices = app.snapshot.active_voice_count;
        mono(ui, "VOICES", &format!("{}", voices));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Output meter (placeholder — peaks from scope buffer).
            let peak = peak_of(&app.snapshot.scope_samples);
            let bar_w = 80.0;
            let bar_h = 4.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(bar_w, bar_h), egui::Sense::hover());
            ui.painter().rect_filled(rect, 1.5, theme::BG_3);
            let fill_w = (peak * bar_w).clamp(0.0, bar_w);
            let fill = egui::Rect::from_min_size(rect.left_top(), egui::vec2(fill_w, bar_h));
            let color = if peak < 0.7 {
                theme::GOOD
            } else if peak < 0.95 {
                theme::WARM
            } else {
                theme::BAD
            };
            ui.painter().rect_filled(fill, 1.5, color);
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("OUT")
                    .color(theme::TEXT_3)
                    .size(10.0)
                    .strong(),
            );
        });
    });
}

fn draw_voices_badge(ui: &mut egui::Ui, count_text: &str) {
    let label = "VOICES";
    let pad_x = 10.0;
    let gap = 6.0;
    let label_font = egui::FontId::proportional(10.0);
    let count_font = egui::FontId::monospace(11.0);

    let label_w = ui
        .painter()
        .layout_no_wrap(label.to_owned(), label_font.clone(), theme::ACCENT_SOFT)
        .size()
        .x;
    let count_w = ui
        .painter()
        .layout_no_wrap(
            count_text.to_owned(),
            count_font.clone(),
            theme::ACCENT_SOFT,
        )
        .size()
        .x;

    let inner_w = label_w + gap + count_w;
    let total = egui::vec2(inner_w + pad_x * 2.0, 22.0);
    let (rect, _) = ui.allocate_exact_size(total, egui::Sense::hover());

    let p = ui.painter_at(rect.expand(2.0));
    p.rect_filled(rect, 11.0, theme::ACCENT_DIM);
    p.rect_stroke(
        rect,
        11.0,
        egui::Stroke::new(1.0, theme::ACCENT),
        egui::StrokeKind::Inside,
    );
    let label_x = rect.left() + pad_x;
    let count_x = rect.right() - pad_x;
    let cy = rect.center().y;
    p.text(
        egui::pos2(label_x, cy),
        egui::Align2::LEFT_CENTER,
        label,
        label_font,
        theme::ACCENT_SOFT,
    );
    p.text(
        egui::pos2(count_x, cy),
        egui::Align2::RIGHT_CENTER,
        count_text,
        count_font,
        theme::ACCENT_SOFT,
    );
}

fn peak_of(scope: &[f32]) -> f32 {
    let mut p = 0.0f32;
    for s in scope.iter() {
        let a = s.abs();
        if a > p {
            p = a;
        }
    }
    p.clamp(0.0, 1.0)
}

/// Apply a factory preset: walk every param and call `set_plain` for any
/// id that matches a key in the preset's `params` object. Missing keys
/// are ignored so older presets still load after a param is added.
fn load_preset(params: &WavetableParams, json: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return;
    };
    let Some(map) = value.get("params").and_then(|v| v.as_object()) else {
        return;
    };
    for i in 0..PARAM_COUNT {
        let p = params.param_at(i);
        if let Some(v) = map.get(p.id()).and_then(|v| v.as_f64()) {
            p.set_plain(v);
        }
    }
}

