//! Drums plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! This is the migration from the old iced-based inline UI onto the same
//! editor infrastructure the wavetable plugin uses. The contents are
//! intentionally minimal placeholder controls — the real layout will be
//! designed in a follow-up. The point of this module is to wire up the
//! `EditorFactory` / `PluginEditor` plumbing so the plugin exposes a
//! floating CLAP editor window.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use resonance_common::registry::InstalledItem;
use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{egui, Editor as RuntimeEditor, EditorApp, EditorOptions};

use crate::download::WorkerHandle;
use crate::kit_loader;
use crate::params::DrumParams;
use crate::KitBridge;

mod download_panel;
mod header;
mod kit_browser;
mod pad_grid;
mod pad_inspector;
mod theme;

/// Reload the kit in place with the current mic selection. Used whenever
/// the user changes a close-mic, overhead-mic, articulation, or any other
/// setup that needs the sample banks to be re-decoded. No-op if there's no
/// kit path yet or the host hasn't activated the plugin.
fn reload_kit(bridge: &KitBridge) {
    let path = match bridge.kit_path.lock().clone() {
        Some(p) => p,
        None => return,
    };
    let sr_bits = bridge.sample_rate.load(Ordering::Acquire);
    if sr_bits == 0 {
        return;
    }
    let target_sr = f32::from_bits(sr_bits);
    let overhead_key = bridge.overhead_setup_key.lock().clone();
    let choices = bridge.pad_choices.lock().clone();
    let articulations = *bridge.articulations.lock();
    kit_loader::spawn_loader(
        path,
        target_sr,
        bridge,
        overhead_key,
        choices,
        articulations,
    );
}

const INITIAL_SIZE: (u32, u32) = (720, 440);
const MIN_SIZE: (u32, u32) = (560, 360);

// ---------------------------------------------------------------------------
// Factory — produced by ResonanceDrums::editor_factory().
// ---------------------------------------------------------------------------

pub struct DrumsEditorFactory {
    params: Arc<DrumParams>,
    bridge: KitBridge,
    download_worker: Arc<WorkerHandle>,
}

impl DrumsEditorFactory {
    pub(crate) fn new(
        params: Arc<DrumParams>,
        bridge: KitBridge,
        download_worker: Arc<WorkerHandle>,
    ) -> Self {
        Self {
            params,
            bridge,
            download_worker,
        }
    }
}

impl EditorFactory for DrumsEditorFactory {
    fn supports(&self, api_name: &str, is_floating: bool) -> bool {
        is_floating && api_name == "wayland"
    }

    fn preferred(&self) -> Option<(&'static str, bool)> {
        Some(("wayland", true))
    }

    fn preferred_size(&self) -> (u32, u32) {
        INITIAL_SIZE
    }

    fn create(&self, api_name: &str, is_floating: bool) -> Option<Box<dyn PluginEditor>> {
        if !self.supports(api_name, is_floating) {
            return None;
        }
        let app = DrumsEditorApp::new(
            self.params.clone(),
            self.bridge.clone(),
            self.download_worker.clone(),
        );
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance Drums".to_string(),
                initial_size: INITIAL_SIZE,
                min_size: MIN_SIZE,
                resizable: true,
            },
        )
        .ok()?;
        Some(Box::new(RuntimeEditorHandle {
            runtime: Some(runtime),
            size: INITIAL_SIZE,
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
        // Not wired into the runtime yet — same TODO as the wavetable editor.
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

struct DrumsEditorApp {
    params: Arc<DrumParams>,
    bridge: KitBridge,
    selected_pad: usize,
    download_worker: Arc<WorkerHandle>,
    download_panel: download_panel::DownloadPanelState,
    /// Cached list of installed drum kits from the shared registry.
    installed_kits: Vec<InstalledItem>,
    /// Frame counter used to periodically refresh the installed-kits cache
    /// (every ~60 frames, roughly once per second at the 100ms repaint rate).
    installed_kits_refresh: u32,
}

impl DrumsEditorApp {
    fn new(params: Arc<DrumParams>, bridge: KitBridge, download_worker: Arc<WorkerHandle>) -> Self {
        let installed_kits = kit_browser::refresh_installed_kits();
        Self {
            params,
            bridge,
            selected_pad: 0,
            download_worker,
            download_panel: download_panel::DownloadPanelState::default(),
            installed_kits,
            installed_kits_refresh: 0,
        }
    }

    /// Refresh the installed-kits cache every ~60 frames. Called once per
    /// frame from `ui()`.
    fn maybe_refresh_installed_kits(&mut self) {
        self.installed_kits_refresh += 1;
        if self.installed_kits_refresh >= 60 {
            self.installed_kits_refresh = 0;
            self.installed_kits = kit_browser::refresh_installed_kits();
        }
    }
}

impl EditorApp for DrumsEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());

        // Periodically refresh the installed-kits cache.
        self.maybe_refresh_installed_kits();

        // Drive a modest repaint so kit-loading status updates flow in.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));

        header::draw(ui, &self.params);

        kit_browser::draw_loader_row(ui, &self.bridge, &mut self.download_panel);
        kit_browser::draw_installed_picker(ui, &self.bridge, &self.installed_kits);

        // Snapshot the catalog once per frame so we don't re-lock on every
        // dropdown. The `.clone()` is cheap compared to the UI work.
        let catalog = self.bridge.catalog.lock().clone();

        kit_browser::draw_overhead_picker(ui, &self.bridge, &catalog);

        ui.separator();

        // Two-column layout: pad list on the left, selected-pad detail on the right.
        pad_grid::draw(ui, &self.bridge, &mut self.selected_pad);

        let selected_pad = self.selected_pad;
        egui::CentralPanel::default().show_inside(ui, |ui| {
            pad_inspector::draw(ui, &self.params, &self.bridge, &catalog, selected_pad);
        });

        if self.download_panel.open {
            download_panel::draw(ui, &mut self.download_panel, &self.download_worker);
        }
    }
}
