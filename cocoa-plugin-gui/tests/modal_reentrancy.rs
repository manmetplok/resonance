//! Modal-reentrancy guard regression test (macos-editor-plan.md §3h).
//!
//! Three plugin editors (amp, drums, ir) call blocking `rfd::FileDialog`
//! from inside `ui()`. On this runtime `ui()` runs on the AppKit main
//! thread, so the dialog's modal run loop re-enters the very run loop the
//! editor paints from: the repaint timer keeps firing (it is scheduled in
//! `NSRunLoopCommonModes`, which include `NSModalPanelRunLoopMode`), and
//! the main dispatch queue keeps draining — so a host `destroy()` can
//! execute *nested inside a frame* whose `ui()` has not returned yet.
//!
//! This test reproduces that shape without an `NSOpenPanel` (a real panel
//! cannot be dismissed programmatically): on its third frame the app's
//! `ui()` spins a nested run loop in `NSModalPanelRunLoopMode` — exactly
//! what `-[NSSavePanel runModal]` does — while the scenario thread calls
//! `Editor::destroy` into it. It asserts the two properties the guard
//! exists for: `ui()` is never re-entered (nested `drawRect:` deliveries
//! are skipped), and a mid-modal teardown completes without crashing the
//! process (the view outlives the registry's references — the keep-alive
//! in `EditorView::paint`). It also pins that a host-initiated destroy
//! does not fire `on_close`.
//!
//! ## Running it
//!
//! Opens a real window, so it needs a logged-in macOS session and a
//! pumped main thread (see `resonance-gate/tests/editor_open_cocoa.rs`
//! for the full why); `harness = false`, hand-rolled `--ignored`:
//!
//! ```text
//! cargo test -p cocoa-plugin-gui --test modal_reentrancy -- --ignored --nocapture
//! ```

#[cfg(target_os = "macos")]
fn main() {
    if !std::env::args().any(|a| a == "--ignored") {
        println!("skipped: opens real windows; pass -- --ignored from a logged-in macOS session");
        return;
    }
    live::run(std::time::Duration::from_secs(30));
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("skipped: the Cocoa modal-reentrancy test only runs on macOS");
}

#[cfg(target_os = "macos")]
mod live {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    use cocoa_plugin_gui::{egui, Editor, EditorApp, EditorOptions};
    use objc2_app_kit::NSModalPanelRunLoopMode;
    use objc2_foundation::{NSDate, NSRunLoop};

    /// How long `ui()` holds the nested modal loop open. Long enough that
    /// the scenario thread's destroy reliably lands inside it, short
    /// enough to keep the test snappy.
    const MODAL_DURATION: Duration = Duration::from_millis(600);

    // Cross-thread test state: written by the app on the main thread,
    // read by the scenario thread.
    static UI_ENTRIES: AtomicU32 = AtomicU32::new(0);
    static IN_UI: AtomicBool = AtomicBool::new(false);
    static REENTERED: AtomicBool = AtomicBool::new(false);
    static MODAL_STARTED: AtomicBool = AtomicBool::new(false);
    static MODAL_ACTIVE: AtomicBool = AtomicBool::new(false);
    static MODAL_FINISHED: AtomicBool = AtomicBool::new(false);
    static ON_CLOSE_FIRED: AtomicBool = AtomicBool::new(false);

    struct ModalApp;

    impl EditorApp for ModalApp {
        fn ui(&mut self, ui: &mut egui::Ui) {
            if IN_UI.swap(true, Ordering::SeqCst) {
                // A nested `drawRect:` got through to `ui()` — the
                // reentrancy guard is broken.
                REENTERED.store(true, Ordering::SeqCst);
            }
            let n = UI_ENTRIES.fetch_add(1, Ordering::SeqCst) + 1;
            ui.label("cpg modal reentrancy test");

            // Third frame: the window is up and settled — stand in for a
            // blocking rfd dialog. `runMode:beforeDate:` in the modal
            // panel mode is the loop `NSSavePanel::runModal` runs; the
            // repaint timer and the main dispatch queue both fire in it.
            if n == 3 && !MODAL_STARTED.swap(true, Ordering::SeqCst) {
                MODAL_ACTIVE.store(true, Ordering::SeqCst);
                let deadline = Instant::now() + MODAL_DURATION;
                let run_loop = NSRunLoop::currentRunLoop();
                while Instant::now() < deadline {
                    let limit = NSDate::dateWithTimeIntervalSinceNow(0.05);
                    // SAFETY: reading a framework-provided extern static.
                    let mode = unsafe { NSModalPanelRunLoopMode };
                    run_loop.runMode_beforeDate(mode, &limit);
                }
                MODAL_ACTIVE.store(false, Ordering::SeqCst);
                MODAL_FINISHED.store(true, Ordering::SeqCst);
            }

            IN_UI.store(false, Ordering::SeqCst);
        }

        fn on_close(&mut self) {
            ON_CLOSE_FIRED.store(true, Ordering::SeqCst);
        }
    }

    pub fn run(budget: Duration) {
        cocoa_plugin_gui::test_support::run_live_test(budget, scenario);
    }

    /// Poll `cond` until it holds or `timeout` passes.
    fn wait_for(cond: impl Fn() -> bool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        cond()
    }

    fn scenario() {
        let editor = Editor::new(
            ModalApp,
            EditorOptions {
                title: "cpg modal reentrancy".to_string(),
                // Ignored by the Cocoa runtime (plan §3c); kept so the
                // options mirror the Wayland twin's.
                app_id: "com.resonance.cpg-modal-test".to_string(),
                initial_size: (480, 240),
                min_size: (200, 150),
                resizable: false,
            },
        )
        .expect("no editor window — run this from a logged-in macOS session");
        editor.show();

        assert!(
            wait_for(|| MODAL_ACTIVE.load(Ordering::SeqCst), Duration::from_secs(10)),
            "ui() never entered its nested modal loop — repaint pacing broken?"
        );
        // Land well inside the modal window, not on its edge.
        std::thread::sleep(Duration::from_millis(100));

        // The destroy is queued onto the main queue (asynchronously from
        // this thread, PLG-05), which the modal loop services: teardown
        // runs nested inside the in-flight frame. Wait for it to land.
        let alive = editor.liveness();
        editor.destroy();
        assert!(
            wait_for(|| !alive.load(Ordering::SeqCst), Duration::from_secs(5)),
            "the queued teardown never ran"
        );
        let mid_modal = !MODAL_FINISHED.load(Ordering::SeqCst);

        assert!(
            mid_modal,
            "destroy was serviced only after the modal loop ended — the \
             modal loop did not drain the main queue, so this test no \
             longer exercises the mid-modal teardown it exists for"
        );
        assert!(
            !REENTERED.load(Ordering::SeqCst),
            "ui() was re-entered during the modal loop — the in_paint \
             reentrancy guard is broken"
        );
        assert!(
            !ON_CLOSE_FIRED.load(Ordering::SeqCst),
            "host-initiated destroy must not fire on_close"
        );

        // Let the main thread unwind out of the nested loop and finish
        // the interrupted frame on the torn-down view — the crash window
        // the keep-alive in EditorView::paint closes.
        assert!(
            wait_for(|| MODAL_FINISHED.load(Ordering::SeqCst), Duration::from_secs(5)),
            "the nested modal loop never finished after destroy"
        );
        std::thread::sleep(Duration::from_millis(200));

        println!(
            "modal reentrancy: {} ui frames, destroy mid-modal ok, no reentry, no on_close",
            UI_ENTRIES.load(Ordering::SeqCst)
        );
    }
}
