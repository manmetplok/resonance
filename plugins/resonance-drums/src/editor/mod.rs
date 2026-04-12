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

use crate::download::WorkerHandle;
use crate::drum_map::{NUM_PADS, PAD_MAPPINGS};
use crate::kit_loader::{self, KitStatus};
use crate::params::DrumParams;
use crate::KitBridge;

mod download_panel;

/// Reload the kit in place with the current mic selection. Used whenever
/// the user changes a close-mic, overhead-mic, or any other setup that
/// needs the sample banks to be re-decoded. No-op if there's no kit
/// path yet or the host hasn't activated the plugin.
fn reload_kit(bridge: &KitBridge) {
    let path = match bridge.kit_path.lock().unwrap().clone() {
        Some(p) => p,
        None => return,
    };
    let sr_bits = bridge.sample_rate.load(Ordering::Acquire);
    if sr_bits == 0 {
        return;
    }
    let target_sr = f32::from_bits(sr_bits);
    let overhead_key = bridge.overhead_setup_key.lock().unwrap().clone();
    let choices = bridge.pad_choices.lock().unwrap().clone();
    kit_loader::spawn_loader(path, target_sr, bridge, overhead_key, choices);
}

mod theme;

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
}

impl DrumsEditorApp {
    fn new(params: Arc<DrumParams>, bridge: KitBridge, download_worker: Arc<WorkerHandle>) -> Self {
        Self {
            params,
            bridge,
            selected_pad: 0,
            download_worker,
            download_panel: download_panel::DownloadPanelState::default(),
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
        let overhead_key = self.bridge.overhead_setup_key.lock().unwrap().clone();
        let choices = self.bridge.pad_choices.lock().unwrap().clone();
        kit_loader::spawn_loader(path, target_sr, &self.bridge, overhead_key, choices);
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

        // Kit loader + global overhead mic selector on the same row.
        ui.horizontal(|ui| {
            if ui.button("Load Kit").clicked() {
                self.load_kit_clicked();
            }
            let dl_btn = egui::Button::new(
                egui::RichText::new("Download Kits")
                    .color(egui::Color32::BLACK)
                    .strong()
                    .size(12.0),
            )
            .fill(theme::ACCENT);
            if ui.add(dl_btn).clicked() {
                self.download_panel.open = true;
            }
            let status = self.bridge.kit_status.lock().unwrap().clone();
            ui.label(
                egui::RichText::new(format_kit_status(&status)).color(theme::TEXT_DIM),
            );
        });

        // Snapshot the catalog once per frame so we don't re-lock on every
        // dropdown. The `.clone()` is cheap compared to the UI work.
        let catalog = self.bridge.catalog.lock().unwrap().clone();
        let overhead_setups = catalog.overhead_setups();

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Overhead:")
                    .color(theme::TEXT_DIM)
                    .size(11.0),
            );
            let current_overhead = self.bridge.overhead_setup_key.lock().unwrap().clone();
            let label = if overhead_setups.is_empty() {
                current_overhead.clone()
            } else {
                current_overhead.clone()
            };
            let mut new_choice: Option<String> = None;
            egui::ComboBox::from_id_salt("overhead_setup")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    if overhead_setups.is_empty() {
                        ui.label(
                            egui::RichText::new("(load a kit first)").color(theme::TEXT_DIM),
                        );
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
                *self.bridge.overhead_setup_key.lock().unwrap() = key;
                reload_kit(&self.bridge);
            }
        });

        ui.separator();

        // Two-column layout: pad list on the left, selected-pad detail on the right.
        #[allow(deprecated)] // SidePanel → Panel::left rename; current API on this egui version
        egui::SidePanel::left("drum_pad_list")
            .default_size(150.0)
            .resizable(false)
            .show_inside(ui, |ui| {
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
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.render_pad_detail(ui, &catalog);
        });

        if self.download_panel.open {
            download_panel::draw(ui, &mut self.download_panel, &self.download_worker);
        }
    }
}

impl DrumsEditorApp {
    fn render_pad_detail(
        &mut self,
        ui: &mut egui::Ui,
        catalog: &crate::mic_catalog::ManifestMicCatalog,
    ) {
        let pad_idx = self.selected_pad;
        let mapping = &PAD_MAPPINGS[pad_idx];
        ui.label(
            egui::RichText::new(mapping.name)
                .size(14.0)
                .strong()
                .color(theme::ACCENT),
        );
        ui.label(
            egui::RichText::new(format!("MIDI note {}", mapping.note))
                .size(10.0)
                .color(theme::TEXT_DIM),
        );
        ui.add_space(6.0);

        let pad = &self.params.pads[pad_idx];

        // Volume + mute on one row.
        ui.horizontal(|ui| {
            let mut vol = pad.volume.value();
            if ui
                .add(egui::Slider::new(&mut vol, 0.0..=1.0).text("Volume"))
                .changed()
            {
                pad.volume.set_value(vol);
            }
            let mut muted = pad.mute.value();
            if ui.checkbox(&mut muted, "Mute").changed() {
                pad.mute.set_plain(if muted { 1.0 } else { 0.0 });
            }
        });

        // Pan slider.
        let mut pan = pad.pan.value();
        if ui
            .add(egui::Slider::new(&mut pan, -1.0..=1.0).text("Pan"))
            .changed()
        {
            pad.pan.set_value(pan);
        }

        ui.add_space(8.0);
        ui.separator();
        ui.label(
            egui::RichText::new("CLOSE MICS")
                .size(10.0)
                .strong()
                .color(theme::TEXT_DIM),
        );

        // Close-mic dropdowns — one per position this pad type uses.
        // Cymbal-class pads have no positions and render a hint instead.
        if mapping.close_mic_positions.is_empty() {
            ui.label(
                egui::RichText::new("No close mic for this drum (overhead only)")
                    .size(11.0)
                    .color(theme::TEXT_DIM),
            );
        } else {
            let mut choices_to_apply: Vec<(String, String)> = Vec::new();
            for position in mapping.close_mic_positions {
                let available = catalog.close_setups(position);
                let current = self
                    .bridge
                    .pad_choices
                    .lock()
                    .unwrap()
                    .get(pad_idx)
                    .and_then(|c| c.close_setups.get(*position).cloned())
                    .or_else(|| available.first().cloned())
                    .unwrap_or_else(|| "(none)".to_string());

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(*position)
                            .size(11.0)
                            .color(theme::TEXT_DIM),
                    );
                    egui::ComboBox::from_id_salt(format!("pad_{}_mic_{}", pad_idx, position))
                        .selected_text(current.clone())
                        .show_ui(ui, |ui| {
                            if available.is_empty() {
                                ui.label(
                                    egui::RichText::new("(load a kit first)")
                                        .color(theme::TEXT_DIM),
                                );
                            }
                            for key in &available {
                                if ui
                                    .selectable_label(*key == current, key.as_str())
                                    .clicked()
                                {
                                    choices_to_apply
                                        .push((position.to_string(), key.clone()));
                                }
                            }
                        });
                });
            }
            if !choices_to_apply.is_empty() {
                {
                    let mut guard = self.bridge.pad_choices.lock().unwrap();
                    for (position, key) in choices_to_apply {
                        guard[pad_idx].close_setups.insert(position, key);
                    }
                }
                reload_kit(&self.bridge);
            }

            // Balance slider only for pads that use two close-mic positions
            // (kick In/Out, snare Top/Btm). Label matches the pad type so
            // the UX is self-explanatory.
            if mapping.close_mic_positions.len() == 2 {
                let (left_label, right_label) = match mapping.close_mic_positions {
                    ["KickIn", "KickOut"] => ("In", "Out"),
                    ["SNTop", "SNBtm"] => ("Top", "Btm"),
                    [a, b] => (a.as_ref() as &str, b.as_ref() as &str),
                    _ => ("A", "B"),
                };
                let mut balance = pad.balance.value();
                if ui
                    .add(
                        egui::Slider::new(&mut balance, 0.0..=1.0)
                            .text(format!("{} \u{2194} {}", left_label, right_label))
                            .custom_formatter(|x, _| format!("{:.2}", x)),
                    )
                    .changed()
                {
                    pad.balance.set_value(balance);
                }
            }
        }

        ui.add_space(8.0);
        ui.separator();
        ui.label(
            egui::RichText::new("OVERHEAD BLEND")
                .size(10.0)
                .strong()
                .color(theme::TEXT_DIM),
        );

        let mut oh = pad.oh_blend.value();
        if ui
            .add(
                egui::Slider::new(&mut oh, 0.0..=1.0)
                    .text("OH amount")
                    .custom_formatter(|x, _| format!("{:.2}", x)),
            )
            .changed()
        {
            pad.oh_blend.set_value(oh);
        }
        ui.label(
            egui::RichText::new(
                "Scales this pad's contribution to the Overhead output port. \
                 Set to 0 to keep the hit out of the overhead bus entirely.",
            )
            .size(10.0)
            .color(theme::TEXT_DIM),
        );
    }
}
