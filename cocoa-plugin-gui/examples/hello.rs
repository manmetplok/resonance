//! Hello-world smoke test for the cocoa-plugin-gui runtime.
//!
//! Opens an egui window in a floating NSWindow and draws a few widgets.
//! Close the window (fires `on_close`) or Ctrl-C to exit.
//!
//!     cargo run -p cocoa-plugin-gui --example hello
//!
//! Standalone difference from plugin hosting: inside a CLAP host the host's
//! NSApplication run loop pumps the main thread; a bare cargo example has
//! none, so this pumps one itself with `NSApplication::run` after creating
//! the editor. That is the *only* thing here a plugin never does.

#[cfg(target_os = "macos")]
fn main() {
    use cocoa_plugin_gui::{egui, Editor, EditorApp, EditorOptions};

    struct HelloApp {
        counter: u32,
        slider: f32,
        text: String,
        close_calls: u32,
    }

    impl EditorApp for HelloApp {
        fn on_close(&mut self) {
            self.close_calls += 1;
            eprintln!("hello: on_close() invoked, call #{}", self.close_calls);
        }

        fn ui(&mut self, ui: &mut egui::Ui) {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.heading("cocoa-plugin-gui :: hello");
                ui.separator();

                ui.label("Phase 3b smoke test. If you can see this, the runtime is working.");

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

    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let mtm = MainThreadMarker::new().expect("hello example must start on the main thread");
    let nsapp = NSApplication::sharedApplication(mtm);
    // A bare executable has no activation policy; Regular gives it a Dock
    // presence so the window can become key. A plugin never does this —
    // the host application already has one.
    nsapp.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    let app = HelloApp {
        counter: 0,
        slider: 50.0,
        text: "type here".to_string(),
        close_calls: 0,
    };

    let editor = Editor::new(
        app,
        EditorOptions {
            title: "cocoa-plugin-gui :: hello".to_string(),
            app_id: "com.resonance.cocoa-plugin-gui.hello".to_string(),
            initial_size: (640, 480),
            min_size: (320, 240),
            resizable: true,
        },
    )
    .expect("Editor::new failed");

    editor.show();
    nsapp.activate();

    // Dev-only cross-thread teardown check (`CPG_TEST_DESTROY_MS=<n>`):
    // destroy the editor from a spawned thread after n ms — the same
    // shape as a CLAP host calling `destroy` from its engine control
    // thread — then exit. Verifies the handle's `Send` dispatch and that
    // the synchronous main-queue teardown returns instead of wedging.
    if let Some(ms) = std::env::var("CPG_TEST_DESTROY_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            eprintln!("hello: destroying editor from thread {:?}", std::thread::current().id());
            editor.destroy();
            eprintln!("hello: destroy returned");
            std::process::exit(0);
        });
        nsapp.run();
        return;
    }

    // Hand the main thread to AppKit — in a plugin the host's run loop does
    // this. The window close button tears the editor down (watch for the
    // on_close line); Ctrl-C exits, mirroring the Wayland example's park.
    // `editor` stays alive across `run()` (which never returns).
    nsapp.run();
    drop(editor);
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("cocoa-plugin-gui's hello example only runs on macOS");
}
