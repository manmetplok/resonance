//! Hello-world smoke test for the wayland-plugin-gui runtime.
//!
//! Opens an egui window on Wayland and draws a few widgets. Close the window
//! or Ctrl-C to exit.
//!
//!     cargo run -p wayland-plugin-gui --example hello

// The runtime only exists on Linux; elsewhere the example compiles to a
// stub main (same shape as cocoa-plugin-gui's hello, mirrored).
#[cfg(target_os = "linux")]
use wayland_plugin_gui::{egui, Editor, EditorApp, EditorOptions};

#[cfg(target_os = "linux")]
struct HelloApp {
    counter: u32,
    slider: f32,
    text: String,
    close_calls: u32,
}

#[cfg(target_os = "linux")]
impl EditorApp for HelloApp {
    fn on_close(&mut self) {
        self.close_calls += 1;
        eprintln!("hello: on_close() invoked, call #{}", self.close_calls);
    }

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

#[cfg(target_os = "linux")]
fn main() {
    let app = HelloApp {
        counter: 0,
        slider: 50.0,
        text: "type here".to_string(),
        close_calls: 0,
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

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("wayland-plugin-gui is Linux-only; on macOS run:");
    eprintln!("    cargo run -p cocoa-plugin-gui --example hello");
}
