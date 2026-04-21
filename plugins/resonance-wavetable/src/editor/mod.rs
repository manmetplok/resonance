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
    /// Most recent audio→UI viz snapshot, refreshed each frame.
    pub(crate) snapshot: VizSnapshot,
}

impl WavetableEditorApp {
    pub fn new(params: Arc<WavetableParams>, viz: Arc<WavetableVizState>) -> Self {
        let snapshot = viz.read_snapshot();
        Self {
            params,
            viz,
            selected_tab: WtTab::Osc,
            selected_osc: 0,
            selected_lfo: 0,
            selected_mod_slot: 0,
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

        egui::Panel::top("wt_tabs")
            .exact_size(32.0)
            .show_inside(ui, |ui| draw_tab_bar(ui, self));

        egui::CentralPanel::default().show_inside(ui, |ui| match self.selected_tab {
            WtTab::Osc => tabs::osc::draw(ui, self),
            WtTab::EnvFilter => tabs::env_filter::draw(ui, self),
            WtTab::Lfo => tabs::lfo::draw(ui, self),
            WtTab::Mod => tabs::mod_matrix::draw(ui, self),
            WtTab::Fx => tabs::fx::draw(ui, self),
        });
    }
}

fn draw_tab_bar(ui: &mut egui::Ui, app: &mut WavetableEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("RESONANCE WAVETABLE")
                .strong()
                .color(theme::ACCENT),
        );
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        tab_button(ui, &mut app.selected_tab, WtTab::Osc, "OSC");
        tab_button(ui, &mut app.selected_tab, WtTab::EnvFilter, "ENV / FILTER");
        tab_button(ui, &mut app.selected_tab, WtTab::Lfo, "LFO");
        tab_button(ui, &mut app.selected_tab, WtTab::Mod, "MOD");
        tab_button(ui, &mut app.selected_tab, WtTab::Fx, "FX");
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        ui.label(egui::RichText::new("Preset").color(theme::TEXT_DIM));
        egui::ComboBox::from_id_salt("wt_preset_combo")
            .width(190.0)
            .selected_text("— select —")
            .show_ui(ui, |ui| {
                for entry in PRESETS {
                    if ui.selectable_label(false, entry.name).clicked() {
                        load_preset(&app.params, entry.json);
                    }
                }
            });

        ui.add_space(16.0);
        ui.separator();
        ui.label(
            egui::RichText::new(format!("voices {}", app.snapshot.active_voice_count))
                .color(theme::TEXT_DIM),
        );
    });
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

fn tab_button(ui: &mut egui::Ui, current: &mut WtTab, this: WtTab, label: &str) {
    let selected = *current == this;
    let color = if selected {
        theme::ACCENT
    } else {
        theme::TEXT_DIM
    };
    if ui
        .add(egui::Button::new(egui::RichText::new(label).color(color).strong()).frame(false))
        .clicked()
    {
        *current = this;
    }
}
