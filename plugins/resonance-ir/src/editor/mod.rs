//! IR plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Mirrors the shape of the drums plugin editor: a Factory that produces
//! a `RuntimeEditorHandle`, which wraps the `wayland_plugin_gui::Editor`
//! and drives an `EditorApp` implementation on the editor thread.
//!
//! The UI is intentionally compact: a header with a "Load IR…" button, a
//! file browser (prev / next across the current directory), a filename /
//! info line, and the two parameter sliders (Dry/Wet and Output Gain).

use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use resonance_plugin::gui::{EditorFactory, PluginEditor};
use wayland_plugin_gui::{egui, Editor as RuntimeEditor, EditorApp, EditorOptions};

use crate::params::IrParams;

mod theme;

const INITIAL_SIZE: (u32, u32) = (520, 360);
const MIN_SIZE: (u32, u32) = (440, 300);

// ---------------------------------------------------------------------------
// Factory — produced by ResonanceIr::editor_factory().
// ---------------------------------------------------------------------------

pub struct IrEditorFactory {
    params: Arc<IrParams>,
    ir_name: Arc<Mutex<String>>,
    ir_info: Arc<Mutex<String>>,
    load_request: Arc<AtomicI32>,
}

impl IrEditorFactory {
    pub(crate) fn new(
        params: Arc<IrParams>,
        ir_name: Arc<Mutex<String>>,
        ir_info: Arc<Mutex<String>>,
        load_request: Arc<AtomicI32>,
    ) -> Self {
        Self {
            params,
            ir_name,
            ir_info,
            load_request,
        }
    }
}

impl EditorFactory for IrEditorFactory {
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
        let app = IrEditorApp::new(
            self.params.clone(),
            self.ir_name.clone(),
            self.ir_info.clone(),
            self.load_request.clone(),
        );
        let runtime = RuntimeEditor::new(
            app,
            EditorOptions {
                title: "Resonance IR".to_string(),
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

    fn set_title(&mut self, _title: &str) {}
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

struct IrEditorApp {
    params: Arc<IrParams>,
    ir_name: Arc<Mutex<String>>,
    ir_info: Arc<Mutex<String>>,
    load_request: Arc<AtomicI32>,
}

impl IrEditorApp {
    fn new(
        params: Arc<IrParams>,
        ir_name: Arc<Mutex<String>>,
        ir_info: Arc<Mutex<String>>,
        load_request: Arc<AtomicI32>,
    ) -> Self {
        Self {
            params,
            ir_name,
            ir_info,
            load_request,
        }
    }

    /// Open a native file picker for a .wav impulse response. On success,
    /// rescan the containing directory so Prev/Next walks the same folder
    /// the user just picked from, then kick off a load.
    fn load_ir_clicked(&self) {
        let picked = rfd::FileDialog::new()
            .add_filter("Impulse response (WAV)", &["wav"])
            .pick_file();
        let Some(path) = picked else { return };
        let path_str = path.to_string_lossy().into_owned();

        let Some(dir) = path.parent() else { return };
        let files = resonance_common::scan_directory(dir, "wav");
        let idx = files.iter().position(|f| f == &path_str).unwrap_or(0);

        *self.params.file_list.lock() = files;
        *self.params.ir_path.lock() = path_str;
        self.params.file_select.set_value(idx as i32);
        self.load_request.store(idx as i32, Ordering::Release);
    }

    /// Step forward or backward through the current directory scan. Wraps
    /// around so walking past the end loops back to the start.
    fn seek_relative(&self, delta: i32) {
        let len = {
            let list = self.params.file_list.lock();
            list.len()
        };
        if len == 0 {
            return;
        }
        let len_i = len as i32;
        let current = self.params.file_select.value();
        let next = (current + delta).rem_euclid(len_i);
        self.params.file_select.set_value(next);
        self.load_request.store(next, Ordering::Release);
    }
}

impl EditorApp for IrEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());

        // Drive a modest repaint so status updates from the loader
        // thread (filename, info) flow into the UI promptly.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));

        // Header: title + Load IR button.
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("RESONANCE IR")
                    .strong()
                    .color(theme::ACCENT)
                    .size(14.0),
            );
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);
            if ui.button("Load IR…").clicked() {
                self.load_ir_clicked();
            }
        });
        ui.separator();

        // Filename + info line.
        let name_text = {
            let name = self.ir_name.lock().clone();
            if name.is_empty() {
                "(No IR loaded)".to_string()
            } else {
                name
            }
        };
        ui.label(
            egui::RichText::new(name_text)
                .size(16.0)
                .color(theme::TEXT),
        );
        let info_text = self.ir_info.lock().clone();
        if !info_text.is_empty() {
            ui.label(
                egui::RichText::new(info_text)
                    .size(11.0)
                    .color(theme::TEXT_DIM),
            );
        } else {
            // Keep the row height stable whether info is present or not.
            ui.label(
                egui::RichText::new(" ")
                    .size(11.0)
                    .color(theme::TEXT_DIM),
            );
        }

        ui.add_space(4.0);

        // Prev / Next navigation + position indicator.
        let list_len = self.params.file_list.lock().len();
        let current_index = self.params.file_select.value() as usize;
        ui.horizontal(|ui| {
            let enabled = list_len > 1;
            ui.add_enabled_ui(enabled, |ui| {
                if ui.button("◀ Prev").clicked() {
                    self.seek_relative(-1);
                }
                if ui.button("Next ▶").clicked() {
                    self.seek_relative(1);
                }
            });
            ui.add_space(8.0);
            let position = if list_len == 0 {
                "(no directory scanned)".to_string()
            } else {
                // Show the current file stem alongside the index so the
                // user can see what they're about to navigate away from.
                let stem = {
                    let list = self.params.file_list.lock();
                    list.get(current_index.min(list_len.saturating_sub(1)))
                        .and_then(|p| Path::new(p).file_stem().map(|s| s.to_string_lossy().into_owned()))
                        .unwrap_or_default()
                };
                format!(
                    "{} / {}  {}",
                    current_index + 1,
                    list_len,
                    stem
                )
            };
            ui.label(
                egui::RichText::new(position)
                    .size(11.0)
                    .color(theme::TEXT_DIM),
            );
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // Parameters.
        let mut dry_wet = self.params.dry_wet.value();
        if ui
            .add(
                egui::Slider::new(&mut dry_wet, 0.0..=1.0)
                    .text("Dry/Wet")
                    .custom_formatter(|x, _| format!("{:.0}%", x * 100.0)),
            )
            .changed()
        {
            self.params.dry_wet.set_value(dry_wet);
        }

        let mut gain = self.params.output_gain.value();
        if ui
            .add(
                egui::Slider::new(&mut gain, 0.1..=10.0)
                    .logarithmic(true)
                    .text("Output Gain")
                    .custom_formatter(|x, _| {
                        let db = 20.0 * x.log10();
                        format!("{:+.1} dB", db)
                    }),
            )
            .changed()
        {
            self.params.output_gain.set_value(gain);
        }
    }
}
