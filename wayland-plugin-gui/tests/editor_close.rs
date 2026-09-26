//! A window closed from its own close button reports that to its owner
//! (PLG-01), and a window the owner destroyed does not.
//!
//! This is what lets the CLAP bridge send `clap_host_gui.closed()`: the
//! runtime raises `Editor::set_closed_callback` after `EditorApp::on_close`,
//! and only then. The click is synthesized through the runtime's own
//! `WPG_TEST_CLOSE_AT` hook, which drives the real CSD close button, so the
//! test is a separate binary: the hook is process-wide and one-shot.
//!
//! Needs a live Wayland session, so it is `#[ignore]`d:
//!
//! ```text
//! cargo test -p wayland-plugin-gui --test editor_close -- --ignored --nocapture
//! ```

#[cfg(target_os = "linux")]
mod live {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Duration;

    use wayland_plugin_gui::{egui, Editor, EditorApp, EditorOptions};

    struct Blank {
        on_close_ran: Arc<AtomicBool>,
    }

    impl EditorApp for Blank {
        fn ui(&mut self, ui: &mut egui::Ui) {
            ui.label("wpg close notification test");
            ui.ctx().request_repaint();
        }
        fn on_close(&mut self) {
            self.on_close_ran.store(true, Ordering::SeqCst);
        }
    }

    fn open(on_close_ran: Arc<AtomicBool>) -> Editor {
        Editor::new(
            Blank { on_close_ran },
            EditorOptions {
                title: "wpg close notification".to_string(),
                app_id: "com.resonance.wpg-close-test".to_string(),
                initial_size: (480, 320),
                min_size: (200, 150),
                resizable: true,
            },
        )
        .expect("no compositor? run this test from a Wayland session")
    }

    #[test]
    #[ignore = "opens real windows; needs a live Wayland session"]
    fn a_self_close_is_reported_and_a_destroy_is_not() {
        // Process-wide (and safe to call in edition 2021): this binary
        // holds this one test, so nothing reads it concurrently.
        std::env::set_var("WPG_TEST_CLOSE_AT", "5");

        // -- the user closes the window from its own close button ---------
        let on_close_ran = Arc::new(AtomicBool::new(false));
        let editor = open(on_close_ran.clone());
        let (tx, rx) = mpsc::channel();
        let on_close_seen = on_close_ran.clone();
        editor.set_closed_callback(move || {
            // The contract: `EditorApp::on_close` has already run.
            let _ = tx.send(on_close_seen.load(Ordering::SeqCst));
        });
        editor.show();
        let on_close_first = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the closed callback never fired after a close-button click");
        assert!(
            on_close_first,
            "the host was told before EditorApp::on_close ran"
        );
        assert!(
            rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "fired twice"
        );
        editor.destroy();

        // -- the owner destroys the window: no report ---------------------
        let calls = Arc::new(AtomicU32::new(0));
        let editor = open(Arc::new(AtomicBool::new(false)));
        let counter = calls.clone();
        editor.set_closed_callback(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        editor.show();
        std::thread::sleep(Duration::from_millis(500));
        editor.destroy();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "a host-initiated destroy was reported back as a close"
        );
    }
}
