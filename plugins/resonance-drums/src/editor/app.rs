//! The actual egui app: state and update/view orchestration for the drums editor.
//!
//! `DrumsEditorApp` is the `EditorApp` the runtime drives each frame. It paints
//! the header (transport / kit name), the loader / installed-kit / overhead
//! pickers, then the pad grid and the per-pad inspector.

use std::sync::Arc;

use resonance_common::registry::InstalledItem;
use wayland_plugin_gui::{egui, EditorApp};

use crate::download::WorkerHandle;
use crate::params::DrumParams;
use crate::KitBridge;

use super::{download_panel, header, kit_browser, pad_grid, pad_inspector, theme};

pub(crate) struct DrumsEditorApp {
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
    pub(super) fn new(
        params: Arc<DrumParams>,
        bridge: KitBridge,
        download_worker: Arc<WorkerHandle>,
    ) -> Self {
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

        // Snapshot the catalog once per frame so we don't re-lock on every
        // dropdown. The `.clone()` is cheap compared to the UI work.
        let catalog = self.bridge.catalog.lock().clone();

        kit_browser::draw_installed_and_overhead_pickers(
            ui,
            &self.bridge,
            &self.installed_kits,
            &catalog,
        );

        ui.add_space(2.0);
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
