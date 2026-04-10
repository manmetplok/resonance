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

use resonance_plugin::gui::{EditorFactory, PluginEditor};
use resonance_plugin::param::Param;
use wayland_plugin_gui::{egui, Editor as RuntimeEditor, EditorApp, EditorOptions};

use crate::drum_map::{NUM_PADS, PAD_MAPPINGS};
use crate::kit_loader::{self, KitStatus};
use crate::params::DrumParams;
use crate::KitBridge;

mod theme;

const INITIAL_SIZE: (u32, u32) = (720, 440);
const MIN_SIZE: (u32, u32) = (560, 360);

// ---------------------------------------------------------------------------
// Factory — produced by ResonanceDrums::editor_factory().
// ---------------------------------------------------------------------------

pub struct DrumsEditorFactory {
    params: Arc<DrumParams>,
    bridge: KitBridge,
}

impl DrumsEditorFactory {
    pub(crate) fn new(params: Arc<DrumParams>, bridge: KitBridge) -> Self {
        Self { params, bridge }
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
        let app = DrumsEditorApp::new(self.params.clone(), self.bridge.clone());
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
}

impl DrumsEditorApp {
    fn new(params: Arc<DrumParams>, bridge: KitBridge) -> Self {
        Self {
            params,
            bridge,
            selected_pad: 0,
        }
    }

    fn load_kit_clicked(&self) {
        // Sync rfd dialog on the editor thread. Blocks briefly while the
        // native file picker is up; the loader thread then does all the
        // heavy work off the UI thread.
        let picked = rfd::FileDialog::new()
            .add_filter("Drum kit manifest", &["json"])
            .pick_file();
        let Some(path) = picked else { return };

        // Refuse to spawn a loader before the host has activated the
        // plugin — without a sample rate we'd decode at the wrong pitch.
        let sr_bits = self.bridge.sample_rate.load(Ordering::Acquire);
        if sr_bits == 0 {
            *self.bridge.kit_status.lock().unwrap() = KitStatus::Error {
                message: "plugin not yet activated by host".to_string(),
            };
            return;
        }
        let target_sr = f32::from_bits(sr_bits);
        kit_loader::spawn_loader(path, target_sr, &self.bridge);
    }
}

fn format_kit_status(status: &KitStatus) -> String {
    match status {
        KitStatus::Empty => "Defaults (no kit loaded)".to_string(),
        KitStatus::Loading { path } => format!(
            "Loading {}…",
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

impl EditorApp for DrumsEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());

        // Drive a modest repaint so kit-loading status updates flow in.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));

        // Header.
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("RESONANCE DRUMS")
                    .strong()
                    .color(theme::ACCENT),
            );
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);
            let master = &self.params.master_volume;
            let mut v = master.value();
            let resp = ui.add(
                egui::Slider::new(&mut v, 0.0..=1.0)
                    .text("Master")
                    .custom_formatter(|x, _| format!("{:.2}", x)),
            );
            if resp.changed() {
                master.set_value(v);
            }
        });
        ui.separator();

        // Kit loader row.
        ui.horizontal(|ui| {
            if ui.button("Load Kit").clicked() {
                self.load_kit_clicked();
            }
            let status = self.bridge.kit_status.lock().unwrap().clone();
            ui.label(
                egui::RichText::new(format_kit_status(&status)).color(theme::TEXT_DIM),
            );
        });
        ui.separator();

        // Pad picker — placeholder list. Real layout (4×3 grid + selected
        // pad detail panel) lands in a follow-up.
        ui.label(
            egui::RichText::new("PADS")
                .size(10.0)
                .strong()
                .color(theme::TEXT_DIM),
        );
        egui::ScrollArea::vertical().show(ui, |ui| {
            for i in 0..NUM_PADS {
                let name = PAD_MAPPINGS[i].name;
                ui.selectable_value(&mut self.selected_pad, i, name);
            }
        });

        ui.add_space(8.0);
        ui.separator();
        ui.label(
            egui::RichText::new(format!("PAD: {}", PAD_MAPPINGS[self.selected_pad].name))
                .size(10.0)
                .strong()
                .color(theme::TEXT_DIM),
        );

        let pad = &self.params.pads[self.selected_pad];

        // Volume.
        let mut vol = pad.volume.value();
        if ui
            .add(egui::Slider::new(&mut vol, 0.0..=1.0).text("Volume"))
            .changed()
        {
            pad.volume.set_value(vol);
        }

        // Pan.
        let mut pan = pad.pan.value();
        if ui
            .add(egui::Slider::new(&mut pan, -1.0..=1.0).text("Pan"))
            .changed()
        {
            pad.pan.set_value(pan);
        }

        // Mute.
        let mut muted = pad.mute.value();
        if ui.checkbox(&mut muted, "Mute").changed() {
            pad.mute.set_plain(if muted { 1.0 } else { 0.0 });
        }
    }
}
