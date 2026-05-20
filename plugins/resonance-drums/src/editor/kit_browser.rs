//! Kit browser / loader controls.
//!
//! Renders the Load Kit / Download Kits buttons, the status line, the
//! installed-kits combo box, and the global overhead-mic selector. Drives
//! the loader thread via [`crate::kit_loader::spawn_loader`].

use std::sync::atomic::Ordering;

use wayland_plugin_gui::egui;

use resonance_common::registry::{self, ContentType, InstalledItem};

use crate::kit_loader::{self, KitStatus};
use crate::mic_catalog::ManifestMicCatalog;
use crate::KitBridge;

use super::download_panel::DownloadPanelState;
use super::{reload_kit, theme};

/// Find the `drum_samples.json` manifest inside a kit directory. The
/// downloaded kits have a nested subdirectory, so we search one level
/// deep as well as the root.
fn find_manifest(kit_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let direct = kit_dir.join("drum_samples.json");
    if direct.exists() {
        return Some(direct);
    }
    // Search one level of subdirectories.
    if let Ok(entries) = std::fs::read_dir(kit_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let nested = entry.path().join("drum_samples.json");
                if nested.exists() {
                    return Some(nested);
                }
            }
        }
    }
    None
}

/// Load a kit from an installed registry entry.
fn load_installed_kit(bridge: &KitBridge, item: &InstalledItem) {
    let kit_dir = std::path::PathBuf::from(&item.path);
    let Some(manifest_path) = find_manifest(&kit_dir) else {
        *bridge.kit_status.lock() = KitStatus::Error {
            message: format!("no drum_samples.json found in {}", kit_dir.display()),
        };
        return;
    };
    let sr_bits = bridge.sample_rate.load(Ordering::Acquire);
    if sr_bits == 0 {
        *bridge.kit_status.lock() = KitStatus::Error {
            message: "plugin not yet activated by host".to_string(),
        };
        return;
    }
    let target_sr = f32::from_bits(sr_bits);
    let overhead_key = bridge.overhead_setup_key.lock().clone();
    let choices = bridge.pad_choices.lock().clone();
    let articulations = *bridge.articulations.lock();
    kit_loader::spawn_loader(
        manifest_path,
        target_sr,
        bridge,
        overhead_key,
        choices,
        articulations,
    );
}

fn load_kit_clicked(bridge: &KitBridge) {
    // Sync rfd dialog on the editor thread. Blocks briefly while the
    // native file picker is up; the loader thread then does all the
    // heavy work off the UI thread.
    let picked = rfd::FileDialog::new()
        .add_filter("Drum kit manifest", &["json"])
        .pick_file();
    let Some(path) = picked else { return };

    // Refuse to spawn a loader before the host has activated the
    // plugin — without a sample rate we'd decode at the wrong pitch.
    let sr_bits = bridge.sample_rate.load(Ordering::Acquire);
    if sr_bits == 0 {
        *bridge.kit_status.lock() = KitStatus::Error {
            message: "plugin not yet activated by host".to_string(),
        };
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

fn format_kit_status(status: &KitStatus) -> String {
    match status {
        KitStatus::Empty => "Defaults (no kit loaded)".to_string(),
        KitStatus::Loading { path } => format!(
            "Loading {}...",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "kit".to_string())
        ),
        KitStatus::Loaded { name, num_pads } => {
            format!("Kit: {name} ({num_pads} pads)")
        }
        KitStatus::Error { message } => {
            let short: String = message.chars().take(80).collect();
            format!("Error: {short}")
        }
    }
}

/// Render the kit loader row: Load Kit button, Download Kits button, and
/// the current load status. Mutates `download_panel.open` when the user
/// opens the download overlay.
pub fn draw_loader_row(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    download_panel: &mut DownloadPanelState,
) {
    ui.horizontal(|ui| {
        if ui.button("Load Kit").clicked() {
            load_kit_clicked(bridge);
        }
        let dl_btn = egui::Button::new(
            egui::RichText::new("Download Kits")
                .color(egui::Color32::BLACK)
                .strong()
                .size(theme::BODY_SIZE + 1.0),
        )
        .fill(theme::ACCENT);
        if ui.add(dl_btn).clicked() {
            download_panel.open = true;
        }
        ui.add_space(4.0);
        let status = bridge.kit_status.lock().clone();
        ui.label(
            egui::RichText::new(format_kit_status(&status))
                .color(theme::TEXT_DIM)
                .size(theme::BODY_SIZE),
        );
    });
}

/// Render the installed-kits + overhead pickers as a single row.
///
/// Putting both pickers on one line keeps the header band compact —
/// the editor is only 440 px tall by default, so every saved row of
/// vertical real estate means more space for the pad inspector below.
/// If the installed list is empty the function still renders the
/// overhead picker, so the user always has access to the global
/// overhead-mic selector after loading a kit via "Load Kit".
pub fn draw_installed_and_overhead_pickers(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    installed_kits: &[InstalledItem],
    catalog: &ManifestMicCatalog,
) {
    let mut kit_to_load: Option<InstalledItem> = None;

    // Derive the "currently loaded" kit name from kit_status so the
    // combo box reflects what's active.
    let current_kit_name = {
        let status = bridge.kit_status.lock();
        match &*status {
            KitStatus::Loaded { name, .. } => name.clone(),
            KitStatus::Loading { path } => path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            _ => String::new(),
        }
    };

    ui.horizontal(|ui| {
        if !installed_kits.is_empty() {
            ui.label(theme::hint_text("Installed:"));
            let selected_text = if current_kit_name.is_empty() {
                "(select a kit)".to_string()
            } else {
                current_kit_name.clone()
            };
            egui::ComboBox::from_id_salt("installed_kits")
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for item in installed_kits {
                        if ui
                            .selectable_label(item.name == current_kit_name, &item.name)
                            .clicked()
                        {
                            kit_to_load = Some(item.clone());
                        }
                    }
                });
            ui.add_space(12.0);
        }

        // Overhead picker — always shown so it's discoverable.
        let overhead_setups = catalog.overhead_setups();
        ui.label(theme::hint_text("Overhead:"));
        let current_overhead = bridge.overhead_setup_key.lock().clone();
        let label = current_overhead.clone();
        let mut new_choice: Option<String> = None;
        egui::ComboBox::from_id_salt("overhead_setup")
            .selected_text(label)
            .show_ui(ui, |ui| {
                if overhead_setups.is_empty() {
                    ui.label(theme::hint_text("(load a kit first)"));
                }
                for key in &overhead_setups {
                    if ui
                        .selectable_label(*key == current_overhead, key.as_str())
                        .clicked()
                    {
                        new_choice = Some(key.clone());
                    }
                }
            });
        if let Some(key) = new_choice {
            *bridge.overhead_setup_key.lock() = key;
            reload_kit(bridge);
        }
    });

    if let Some(item) = kit_to_load {
        load_installed_kit(bridge, &item);
    }
}

/// Refresh the installed-kits cache from the registry.
pub fn refresh_installed_kits() -> Vec<InstalledItem> {
    registry::list_installed(&ContentType::Drumkit)
}
