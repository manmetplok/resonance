//! The editor open/close round trip, against a real compositor (ba todo #1375).
//!
//! Nothing in the workspace used to drive
//! [`EditorFactory::create`](resonance_plugin::gui::EditorFactory::create)
//! -> `show` -> `size` -> `set_size` -> `hide` -> drop. That is the one
//! stretch of plugin code where "it compiles" means nothing:
//! `RuntimeEditorHandle` (resonance-plugin/src/editor_host.rs, shared by
//! all 11 plugins since ba todo #1336) holds its runtime in an `Option`
//! purely so `Drop` can move it out — `Editor::destroy` consumes the
//! editor while `Drop` only gets `&mut self`. Get that wrong and the
//! build is still clean; the host simply wedges in `destroy()`'s
//! thread-join the first time a user closes a plugin window. #1336 was
//! signed off by an ad-hoc harness that opened all 11 windows by hand
//! and was then thrown away. This is that harness, kept.
//!
//! One plugin is enough. Since #1336 the handle is shared, so the code
//! under test here is the code all 11 run; the gate is the cheapest way
//! in (an 820x320 window over three atomics, no analyzer, no FFT). The
//! plugin crates are `crate-type = ["cdylib", "lib"]` and each exports
//! `clap_entry`, so they cannot co-link into one test binary anyway.
//!
//! ## Running it
//!
//! The live test opens a real window and needs a Wayland session, so it
//! is `#[ignore]`d — the same gate `wayland-plugin-gui/tests/editor_size.rs`
//! uses for its live check, and the one `scripts/run-tests.py` honours
//! for free (it runs each binary with no extra libtest flags, so an
//! ignored test is skipped without anyone having to remember to *unset*
//! anything). An env-var gate fails the wrong way round: a stray
//! `export` in a shell profile would quietly arm it on a headless box.
//!
//! ```text
//! cargo test -p resonance-gate --test editor_open -- --ignored --nocapture
//! ```
//!
//! The rest of the file — factory negotiation and the teardown guard
//! itself — is pure and runs in the default headless suite.

#![cfg(feature = "editor")]

use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use resonance_gate::editor::GateEditorFactory;
use resonance_gate::params::GateParams;
use resonance_gate::viz::GateViz;
use resonance_plugin::gui::{EditorFactory, PluginEditor};
use resonance_plugin::presets::PresetSession;

/// How long teardown may take before we call it a hang.
///
/// Measured at ~2 ms on this machine, so this is a ~2500x margin: it is
/// not a performance budget, it is the line between "closed" and "the
/// host is wedged". It has to be *some* finite number, because a test
/// that hangs forever is not a failing test, it is a stuck CI job.
const TEARDOWN_BUDGET: Duration = Duration::from_secs(5);

/// How long a size we asked for gets to show up before we call it lost.
const SIZE_BUDGET: Duration = Duration::from_secs(2);

/// A size distinct from the factory's preferred one and above the
/// editor's declared minimum (640x260), so the resize is a real change
/// the runtime is allowed to make.
const RESIZED: (u32, u32) = (940, 400);

fn factory() -> GateEditorFactory {
    GateEditorFactory::new(
        Arc::new(GateParams::default()),
        GateViz::new(),
        PresetSession::new(),
    )
}

/// Poll the handle until `done` accepts what it reports, or the budget
/// runs out; return whatever it last said. Same shape as the poll loop
/// in `wayland-plugin-gui/tests/editor_size.rs` — the runtime applies
/// commands on the editor thread, so every size answer is eventual.
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

/// Poll until the reported size stops moving, so the window is really
/// mapped and configured before we start asserting on it.
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
/// finished inside `budget`.
///
/// The failure mode this whole file exists for is a teardown that never
/// returns, and you cannot assert on that from the thread that is stuck
/// in it. So the drop runs somewhere it is allowed to hang and the test
/// thread watches the clock. If it does hang, the watcher returns
/// `false`, the test fails normally, and libtest tears the process down
/// with the wedged thread still parked in `destroy()`.
fn dropped_within(editor: Box<dyn PluginEditor>, budget: Duration) -> bool {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("editor-teardown".to_string())
        .spawn(move || {
            drop(editor);
            // Only reached if the drop returned; the receiver reads the
            // absence of this as the hang.
            let _ = tx.send(());
        })
        .expect("could not spawn the teardown watchdog thread");
    rx.recv_timeout(budget).is_ok()
}

// ---------------------------------------------------------------------------
// Pure: factory negotiation (no compositor, runs in the default suite)
// ---------------------------------------------------------------------------

/// `create` must refuse an api/floating combo it does not support
/// *without* trying to open anything — this is the branch that keeps a
/// non-Wayland host from getting a window it cannot embed, and it is
/// also what makes this part of the file safe to run headless.
#[test]
fn create_refuses_an_unsupported_api() {
    let factory = factory();
    assert!(factory.supports("wayland", true));
    assert!(!factory.supports("wayland", false), "embedded is not supported");
    assert!(!factory.supports("x11", true));

    assert!(factory.create("x11", true).is_none());
    assert!(factory.create("wayland", false).is_none());
}

#[test]
fn the_preferred_api_is_one_the_factory_supports() {
    let factory = factory();
    let (api, floating) = factory.preferred().expect("no preferred api reported");
    assert!(
        factory.supports(api, floating),
        "factory prefers {api}/floating={floating} but does not support it"
    );

    let (w, h) = factory.preferred_size();
    assert!(w > 0 && h > 0, "preferred size {w}x{h} would fail to map");
}

// ---------------------------------------------------------------------------
// The teardown guard's own failure detection
// ---------------------------------------------------------------------------

/// A `Drop` that never returns is what a mis-written `RuntimeEditorHandle`
/// produces, so prove the watchdog actually reports one rather than
/// waiting on it. Pure — no compositor, no window.
#[test]
fn the_teardown_guard_catches_a_drop_that_never_returns() {
    struct Wedged;

    impl PluginEditor for Wedged {
        fn show(&mut self) {}
        fn hide(&mut self) {}
        fn size(&self) -> (u32, u32) {
            (0, 0)
        }
        fn set_size(&mut self, _: u32, _: u32) -> bool {
            false
        }
        fn can_resize(&self) -> bool {
            false
        }
    }

    impl Drop for Wedged {
        fn drop(&mut self) {
            // Stands in for `Editor::destroy` joining a thread that
            // never exits. The watchdog abandons it; the process reaps
            // it on exit.
            loop {
                std::thread::park();
            }
        }
    }

    assert!(
        !dropped_within(Box::new(Wedged), Duration::from_millis(250)),
        "the watchdog reported a teardown that never happened"
    );
}

// ---------------------------------------------------------------------------
// Live: a real window on a real compositor
// ---------------------------------------------------------------------------

/// The whole round trip a host performs: negotiate, create, show, read
/// the size, resize, hide, close.
///
/// ## On the size assertions
///
/// A tiling compositor owns window geometry, and this machine's does:
/// the gate asks to open at 820x320 and gets mapped at whatever the tile
/// is (1571x856 here). So `mapped == preferred_size()` is *not* a
/// correctness statement about the plugin — it is a statement about the
/// window manager, and it is false on half the desktops we support. What
/// the plugin genuinely controls is the *request*: `set_size` is
/// accepted, and from then on the handle reports the size that was
/// asked for (ba todo #1337 — `size()` reports what the window is, and
/// a plugin-initiated resize is one of the things it can be). So the
/// preferred size is asserted by *asking for it* rather than by assuming
/// the compositor granted it on map, and the as-mapped size is only
/// required to be a mappable one.
#[test]
#[ignore = "opens a real window; needs a live Wayland session (see the module docs)"]
fn the_editor_opens_resizes_and_closes() {
    let factory = factory();
    let preferred = factory.preferred_size();

    let mut editor = factory
        .create("wayland", true)
        .expect("no editor window — run this test from a Wayland session");

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

    // The size the plugin wants, asked for explicitly so the compositor's
    // opening geometry is not part of the assertion.
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

    // Hiding stops the window drawing; it must not lose the geometry the
    // host would persist.
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
