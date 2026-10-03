//! The editor open/close round trip on macOS (macos-editor-plan.md §3f).
//!
//! The Cocoa twin of `tests/editor_open.rs`: the same host round trip
//! (`create` -> `show` -> size -> `set_size` -> `hide` -> drop) through
//! the same `GateEditorFactory`, now reaching `cocoa-plugin-gui` via the
//! `RuntimeEditor` cfg pair. The failure mode it guards is also the same
//! teardown wedge — here a main-thread-dispatch deadlock in
//! `Editor::destroy` instead of a thread-join hang — so the same
//! wall-clock watchdog discipline applies (see that file's module docs
//! for the full rationale; it is not repeated here).
//!
//! ## Why this is its own `harness = false` binary
//!
//! libtest runs `#[test]` fns on worker threads and parks the main
//! thread in its own scheduler, so nobody services the AppKit dispatch
//! queue and the first `Editor::new` blocks forever — the test would
//! *hang*, not fail. The real `main()` here hands the main thread to
//! `NSApplication` (via `cocoa_plugin_gui::test_support`) and runs the
//! scenario on a worker thread: exactly a CLAP host's split, run loop on
//! main, gui-extension calls from the engine control thread. The Wayland
//! twin keeps libtest because its runtime owns a dedicated editor
//! thread; this one cannot, so the two live in separate binaries and the
//! pure negotiation/watchdog tests stay in `editor_open.rs`, which runs
//! on every platform.
//!
//! ## Running it
//!
//! Opens a real window, so it needs a logged-in macOS session. Without
//! `--ignored` it skips and exits 0 — the same convention as its
//! `#[ignore]`d twin, hand-rolled because there is no libtest here, and
//! what keeps the default `scripts/run-tests.py` sweep headless:
//!
//! ```text
//! cargo test -p resonance-gate --test editor_open_cocoa -- --ignored --nocapture
//! ```

#[cfg(all(target_os = "macos", feature = "editor"))]
fn main() {
    if !std::env::args().any(|a| a == "--ignored") {
        println!("skipped: opens a real window; pass -- --ignored from a logged-in macOS session");
        return;
    }
    // Budget for the whole run — creation, two resizes, teardown. The
    // round trip measures in milliseconds; this is the wedge line, not a
    // performance target.
    live::run(std::time::Duration::from_secs(30));
}

#[cfg(not(all(target_os = "macos", feature = "editor")))]
fn main() {
    println!("skipped: the Cocoa editor round trip only runs on macOS with the editor feature");
}

#[cfg(all(target_os = "macos", feature = "editor"))]
mod live {
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use resonance_gate::editor::GateEditorFactory;
    use resonance_gate::params::GateParams;
    use resonance_gate::viz::GateViz;
    use resonance_plugin::gui::{EditorFactory, PluginEditor};
    use resonance_plugin::presets::PresetSession;

    /// How long teardown may take before we call it a hang — the same
    /// ~2500x margin as the Wayland twin's `TEARDOWN_BUDGET`.
    const TEARDOWN_BUDGET: Duration = Duration::from_secs(5);

    /// How long a size we asked for gets to show up before we call it lost.
    const SIZE_BUDGET: Duration = Duration::from_secs(2);

    /// A size distinct from the factory's preferred one and above the
    /// editor's declared minimum (640x260), so the resize is a real
    /// change the runtime is allowed to make.
    const RESIZED: (u32, u32) = (940, 400);

    pub fn run(budget: Duration) {
        cocoa_plugin_gui::test_support::run_live_test(budget, scenario);
    }

    fn factory() -> GateEditorFactory {
        GateEditorFactory::new(
            Arc::new(GateParams::default()),
            GateViz::new(),
            PresetSession::new(),
            resonance_plugin::EditAnnouncer::new(),
        )
    }

    /// Poll the handle until `done` accepts what it reports, or the
    /// budget runs out; return whatever it last said. The main thread
    /// applies commands asynchronously, so every size answer is eventual
    /// — same shape as the Wayland twin's poll loop.
    fn wait_until(editor: &dyn PluginEditor, done: impl Fn((u32, u32)) -> bool) -> (u32, u32) {
        let deadline = Instant::now() + SIZE_BUDGET;
        loop {
            let seen = editor.size();
            if done(seen) || Instant::now() >= deadline {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Poll until the reported size stops moving, so the window is
    /// really mapped and configured before we start asserting on it.
    fn settle(editor: &dyn PluginEditor) -> (u32, u32) {
        let deadline = Instant::now() + SIZE_BUDGET;
        let mut last = editor.size();
        let mut unchanged = 0;
        while Instant::now() < deadline && unchanged < 4 {
            std::thread::sleep(Duration::from_millis(50));
            let now = editor.size();
            if now == last {
                unchanged += 1;
            } else {
                unchanged = 0;
                last = now;
            }
        }
        last
    }

    /// Drop `editor` on a scratch thread and report whether the drop
    /// finished inside `budget` — the Wayland twin's watchdog, verbatim:
    /// a wedged `destroy()` cannot be asserted on from the thread stuck
    /// in it.
    fn dropped_within(editor: Box<dyn PluginEditor>, budget: Duration) -> bool {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("editor-teardown".to_string())
            .spawn(move || {
                drop(editor);
                let _ = tx.send(());
            })
            .expect("could not spawn the teardown watchdog thread");
        rx.recv_timeout(budget).is_ok()
    }

    /// The whole round trip a host performs: negotiate, create, show,
    /// read the size, resize, hide, close.
    ///
    /// The size assertions follow the Wayland twin's rule — assert the
    /// sizes the plugin *asked for*, never the size the window happened
    /// to map at. AppKit generally grants a floating window its frame,
    /// but zoom and full-screen are the compositor-imposed sizes of this
    /// platform, so the opening geometry stays out of the assertions.
    fn scenario() {
        let factory = factory();
        let preferred = factory.preferred_size();

        let mut editor = factory
            .create(resonance_plugin::editor_host::native_api(), true)
            .expect("no editor window — run this from a logged-in macOS session");

        editor.show();
        let mapped = settle(&*editor);
        assert!(
            mapped.0 > 0 && mapped.1 > 0,
            "the window mapped at {mapped:?}, which cannot be rendered"
        );
        println!("preferred {preferred:?} -> mapped {mapped:?}");

        assert!(
            editor.can_resize(),
            "the gate's editor declares resizable: true"
        );

        // The size the plugin wants, asked for explicitly so the opening
        // geometry is not part of the assertion.
        assert!(
            editor.set_size(preferred.0, preferred.1),
            "the runtime refused the plugin's own preferred size {preferred:?}"
        );
        let at_preferred = wait_until(&*editor, |seen| seen == preferred);
        assert_eq!(
            at_preferred, preferred,
            "asked for the preferred size, handle still reports {at_preferred:?}"
        );

        // And a second, different size, so the first result cannot be a
        // handle that simply never changed.
        assert!(
            editor.set_size(RESIZED.0, RESIZED.1),
            "the runtime refused a resize to {RESIZED:?}"
        );
        let at_resized = wait_until(&*editor, |seen| seen == RESIZED);
        assert_eq!(
            at_resized, RESIZED,
            "resize to {RESIZED:?} was not honoured; handle reports {at_resized:?}"
        );

        // Hiding stops the window drawing; it must not lose the geometry
        // the host would persist.
        editor.hide();
        assert_eq!(
            editor.size(),
            RESIZED,
            "hiding the window changed the size the host reads back"
        );

        // The point of the whole test.
        let started = Instant::now();
        assert!(
            dropped_within(editor, TEARDOWN_BUDGET),
            "closing the editor did not complete within {TEARDOWN_BUDGET:?} — \
             teardown is wedged (RuntimeEditorHandle::drop / Editor::destroy)"
        );
        println!("teardown completed in {:?}", started.elapsed());
    }
}
