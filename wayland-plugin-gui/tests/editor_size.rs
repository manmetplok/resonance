//! Compositor-driven size feedback (ba todo #1337).
//!
//! `Editor::get_size` used to return the last size *requested*, so a
//! user resizing the editor window through the compositor left the host
//! persisting — and next session restoring — a size the window had not
//! had since it opened. The editor thread now publishes every size it
//! applies into a [`SharedSize`] cell that the handle reads.
//!
//! The pure cell is tested here unconditionally. The end-to-end check
//! needs a real compositor and puts a real window on the user's screen,
//! so it is `#[ignore]`d and run by hand:
//!
//! ```text
//! cargo test -p wayland-plugin-gui --test editor_size -- --ignored --nocapture
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use wayland_plugin_gui::SharedSize;

#[test]
fn the_cell_starts_at_the_requested_initial_size() {
    let size = SharedSize::new((960, 540));
    assert_eq!(size.get(), (960, 540));
}

/// Both dimensions live in one atomic precisely so a reader can never
/// see the width of one size next to the height of another.
#[test]
fn both_dimensions_update_together() {
    let size = SharedSize::new((800, 600));
    size.set((1280, 720));
    assert_eq!(size.get(), (1280, 720));
    size.set((1, 4_000_000_000));
    assert_eq!(size.get(), (1, 4_000_000_000));
}

/// The editor thread writes, the handle reads: a resize applied on the
/// editor thread has to be visible from the thread holding the handle.
#[test]
fn a_write_from_another_thread_is_observed_by_the_handle() {
    let editor_thread_view = SharedSize::new((800, 600));
    let handle_view = editor_thread_view.clone();
    let done = Arc::new(AtomicBool::new(false));
    let done_writer = done.clone();

    let writer = std::thread::spawn(move || {
        editor_thread_view.set((1024, 768));
        done_writer.store(true, Ordering::Release);
    });
    writer.join().expect("writer thread panicked");

    assert!(done.load(Ordering::Acquire));
    assert_eq!(handle_view.get(), (1024, 768));
}

// ---------------------------------------------------------------------------
// End-to-end, against a live compositor (ignored by default)
// ---------------------------------------------------------------------------

// Linux-only: it drives the real Wayland `Editor` (and `hyprctl`); the
// pure `SharedSize` tests above still run on every platform.
#[cfg(all(test, target_os = "linux"))]
mod live {
    use std::time::{Duration, Instant};

    use wayland_plugin_gui::{egui, Editor, EditorApp, EditorOptions};

    struct Blank;

    impl EditorApp for Blank {
        fn ui(&mut self, ui: &mut egui::Ui) {
            ui.label("wpg size feedback test");
        }
    }

    const APP_ID: &str = "com.resonance.wpg-size-test";

    /// The live tests run one at a time: the fd count below would
    /// otherwise see the other test's window.
    static LIVE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serialize() -> std::sync::MutexGuard<'static, ()> {
        LIVE.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn open(initial: (u32, u32)) -> Editor {
        let editor = Editor::new(
            Blank,
            EditorOptions {
                title: "wpg size feedback".to_string(),
                app_id: APP_ID.to_string(),
                initial_size: initial,
                min_size: (200, 150),
                resizable: true,
            },
        )
        .expect("no compositor? run this test from a Wayland session");
        editor.show();
        editor
    }

    /// Poll the handle until it reports `target` or the deadline passes,
    /// then return whatever it last said.
    fn settle(editor: &Editor, target: (u32, u32)) -> (u32, u32) {
        wait_until(editor, |seen| seen == target)
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

    /// Resize the window from *outside* the process — the compositor
    /// doing to it exactly what a user dragging its edge does. Hyprland
    /// specific, which is fine for a manually-run check; returns false
    /// if `hyprctl` isn't there.
    fn compositor_resize(target: (u32, u32)) -> bool {
        let run = |args: &[&str]| {
            std::process::Command::new("hyprctl")
                .args(args)
                .output()
                .ok()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        let selector = format!("class:^({APP_ID})$");
        // Tiled windows ignore an explicit pixel size, so float it first.
        run(&["dispatch", "setfloating", &selector]);
        run(&[
            "dispatch",
            "resizewindowpixel",
            &format!("exact {} {},{selector}", target.0, target.1),
        ])
    }

    /// The contract, end to end: a window the *compositor* resized
    /// reports its new size through the handle — which is what the
    /// plugin, and through it the host, reads.
    ///
    /// Run at two different default sizes, the way two plugins have
    /// different defaults (the EQ opens at 960x540, the compressor at
    /// 720x480).
    ///
    /// The assertion is that the reported size *follows the window*, not
    /// that it equals the requested target: a tiling compositor decides
    /// the geometry itself and will hand out a size nobody asked for
    /// (which is exactly the case the old code got wrong).
    #[test]
    #[ignore = "opens real windows; needs a live Wayland compositor (hyprctl)"]
    fn a_compositor_resize_is_reported_by_the_handle() {
        let _live = serialize();
        for (initial, target) in [((960, 540), (1100, 620)), ((720, 480), (860, 560))] {
            let editor = open(initial);
            // Whatever the compositor configured the window to on map —
            // under a tiling layout that is already not `initial`.
            let mapped = stable_size(&editor);
            assert!(
                compositor_resize(target),
                "could not drive the compositor (is hyprctl on PATH?)"
            );
            let seen = wait_until(&editor, |s| s != mapped);

            // A resize the plugin itself asks for is published the same
            // way, so `get_size` answers consistently whoever initiated
            // the change.
            let mut editor = editor;
            editor.set_size(initial.0, initial.1).expect("resize refused");
            let requested = settle(&editor, initial);
            editor.destroy();

            println!("opened {initial:?} -> mapped {mapped:?} -> compositor {seen:?} -> requested {requested:?}");
            assert_ne!(
                seen, mapped,
                "the handle still reports the old size after a compositor resize"
            );
            assert_eq!(requested, initial, "a requested resize was not reported");
        }
    }

    /// Open file descriptors on DRM device nodes (`/dev/dri/*`).
    fn dri_fds() -> usize {
        std::fs::read_dir("/proc/self/fd")
            .map(|dir| {
                dir.filter_map(|e| std::fs::read_link(e.ok()?.path()).ok())
                    .filter(|target| target.starts_with("/dev/dri"))
                    .count()
            })
            .unwrap_or(0)
    }

    /// Opening and closing an editor must give back what EGL took (PLG-02).
    ///
    /// Each editor initializes an EGL display on its own `wl_display`,
    /// which makes the driver open a render node and build a screen. Until
    /// the display was terminated on teardown, every open leaked one DRM
    /// fd (plus the driver's allocations), and a reopen could be handed the
    /// previous editor's stale display. The 20th editor must still paint.
    #[test]
    #[ignore = "opens real windows; needs a live Wayland compositor"]
    fn reopening_the_editor_does_not_leak_driver_state() {
        let _live = serialize();
        const CYCLES: usize = 20;
        // One warm-up cycle: the first EGL load may keep process-wide
        // driver state (the loaded DRI driver itself) for good.
        open((480, 320)).destroy();
        let before = dri_fds();
        for _ in 0..CYCLES {
            let editor = open((480, 320));
            stable_size(&editor);
            editor.destroy();
        }
        let after = dri_fds();
        println!("/dev/dri fds: {before} before, {after} after {CYCLES} open/close cycles");
        assert!(
            after <= before,
            "{} DRM fds leaked over {CYCLES} editor open/close cycles",
            after - before
        );

        // …and the next editor still works: it maps, paints and follows
        // a resize (a stale display would fail here, or kill the thread).
        let mut editor = open((480, 320));
        stable_size(&editor);
        editor.set_size(520, 360).expect("resize refused: editor thread died");
        assert_eq!(settle(&editor, (520, 360)), (520, 360));
        editor.destroy();
    }
}
