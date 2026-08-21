//! Requested-size behavior of the Cocoa runtime (macos-editor-plan.md §3f).
//!
//! The macOS counterpart of `wayland-plugin-gui/tests/editor_size.rs`,
//! narrowed to what this platform lets a test assert honestly: sizes the
//! caller **requested**. The Wayland test's warning about compositor-
//! imposed geometry maps to zoom and full-screen here — a user (or the
//! window server) can hand the window a frame nobody asked for — so the
//! opening geometry is never asserted, only that `get_size` follows each
//! `set_size` request and survives `hide`. Sizes are logical points
//! throughout: on a Retina display a runtime that leaked
//! `backingScaleFactor` into the handle would report double what was
//! requested, which these assertions would catch.
//!
//! (The pure `SharedSize` cell tests live in the Wayland twin and run on
//! every platform already; they are not duplicated here.)
//!
//! ## Running it
//!
//! Opens real windows, so it needs a logged-in macOS session — and a
//! pumped main thread, which libtest cannot provide (see
//! `resonance-gate/tests/editor_open_cocoa.rs` for the full why), so
//! this is a `harness = false` binary using the same hand-rolled
//! `--ignored` convention as its `#[ignore]`d Wayland twin:
//!
//! ```text
//! cargo test -p cocoa-plugin-gui --test editor_size -- --ignored --nocapture
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
    println!("skipped: the Cocoa size test only runs on macOS");
}

#[cfg(target_os = "macos")]
mod live {
    use std::time::{Duration, Instant};

    use cocoa_plugin_gui::{egui, Editor, EditorApp, EditorOptions};

    struct Blank;

    impl EditorApp for Blank {
        fn ui(&mut self, ui: &mut egui::Ui) {
            ui.label("cpg size feedback test");
        }
    }

    /// How long teardown may take before we call it a hang — same margin
    /// as the lifecycle test's `TEARDOWN_BUDGET`.
    const TEARDOWN_BUDGET: Duration = Duration::from_secs(5);

    pub fn run(budget: Duration) {
        cocoa_plugin_gui::test_support::run_live_test(budget, scenario);
    }

    fn open(initial: (u32, u32)) -> Editor {
        let editor = Editor::new(
            Blank,
            EditorOptions {
                title: "cpg size feedback".to_string(),
                // Ignored by the Cocoa runtime (plan §3c); kept so the
                // options mirror the Wayland twin's.
                app_id: "com.resonance.cpg-size-test".to_string(),
                initial_size: initial,
                min_size: (200, 150),
                resizable: true,
            },
        )
        .expect("no editor window — run this from a logged-in macOS session");
        editor.show();
        editor
    }

    /// Poll the handle until `done` accepts what it reports, or five
    /// seconds pass.
    fn wait_until(editor: &Editor, done: impl Fn((u32, u32)) -> bool) -> (u32, u32) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let seen = editor.get_size();
            if done(seen) || Instant::now() >= deadline {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The size the window settles at: poll until the reported size
    /// stops changing (or two seconds pass).
    fn stable_size(editor: &Editor) -> (u32, u32) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut last = editor.get_size();
        let mut unchanged = 0;
        while Instant::now() < deadline && unchanged < 5 {
            std::thread::sleep(Duration::from_millis(50));
            let now = editor.get_size();
            if now == last {
                unchanged += 1;
            } else {
                unchanged = 0;
                last = now;
            }
        }
        last
    }

    /// Destroy `editor` on a scratch thread under a wall-clock budget —
    /// the same guard as the lifecycle test, because a size test that
    /// wedges in teardown is still a wedge.
    fn destroyed_within(editor: Editor, budget: Duration) -> bool {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("editor-teardown".to_string())
            .spawn(move || {
                editor.destroy();
                let _ = tx.send(());
            })
            .expect("could not spawn the teardown watchdog thread");
        rx.recv_timeout(budget).is_ok()
    }

    /// The contract: `get_size` follows every size the caller asks for —
    /// through two requests (so one success cannot be a handle that never
    /// changed), across `hide`, at two different opening sizes (the way
    /// two plugins have different defaults), in logical points.
    fn scenario() {
        for (initial, target) in [((960, 540), (1100, 620)), ((720, 480), (860, 560))] {
            let mut editor = open(initial);
            // Whatever the window server configured on map — reported,
            // never asserted (zoom/full-screen own this on macOS the way
            // a tiling compositor does on Wayland).
            let mapped = stable_size(&editor);

            editor.set_size(target.0, target.1).expect("resize refused");
            let at_target = wait_until(&editor, |s| s == target);
            assert_eq!(
                at_target, target,
                "asked for {target:?}, handle still reports {at_target:?}"
            );

            editor.set_size(initial.0, initial.1).expect("resize refused");
            let back = wait_until(&editor, |s| s == initial);
            assert_eq!(back, initial, "a second requested resize was not reported");

            // Hiding must not lose the geometry the host would persist.
            editor.hide();
            assert_eq!(
                editor.get_size(),
                initial,
                "hiding the window changed the size the host reads back"
            );

            println!("opened {initial:?} -> mapped {mapped:?} -> requested {target:?} -> back to {initial:?}");
            assert!(
                destroyed_within(editor, TEARDOWN_BUDGET),
                "destroy did not complete within {TEARDOWN_BUDGET:?} — teardown is wedged"
            );
        }
    }
}
