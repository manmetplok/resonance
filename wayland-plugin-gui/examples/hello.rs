//! Hello-world smoke test for the wayland-plugin-gui runtime.
//!
//! Opens an egui window on Wayland and draws a few widgets. Close the window
//! or Ctrl-C to exit.
//!
//!     cargo run -p wayland-plugin-gui --example hello

use wayland_plugin_gui::{egui, Editor, EditorApp, EditorOptions};

struct HelloApp {
    counter: u32,
    slider: f32,
    text: String,
}

impl EditorApp for HelloApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.heading("wayland-plugin-gui :: hello");
            ui.separator();

            ui.label("Phase 0 smoke test. If you can see this, the runtime is working.");

            ui.horizontal(|ui| {
                if ui.button("click me").clicked() {
                    self.counter += 1;
                }
                ui.label(format!("clicks: {}", self.counter));
            });

            ui.add(egui::Slider::new(&mut self.slider, 0.0..=100.0).text("slider"));

            ui.horizontal(|ui| {
                ui.label("text input:");
                ui.text_edit_singleline(&mut self.text);
            });

            ui.separator();
            ui.label(format!(
                "pixels_per_point: {:.2}",
                ui.ctx().pixels_per_point()
            ));
        });
    }
}

fn main() {
    let app = HelloApp {
        counter: 0,
        slider: 50.0,
        text: "type here".to_string(),
    };

    let editor = Editor::new(
        app,
        EditorOptions {
            title: "wayland-plugin-gui :: hello".to_string(),
            app_id: "com.resonance.wayland-plugin-gui.hello".to_string(),
            initial_size: (640, 480),
            min_size: (320, 240),
            resizable: true,
        },
    )
    .expect("Editor::new failed");

    editor.show();

    // Block the main thread. The editor owns its own thread; Ctrl-C or the
    // window close button tears everything down.
    loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}
