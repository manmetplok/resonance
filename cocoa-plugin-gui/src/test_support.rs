//! Main-thread pump for the live (`--ignored`-style) integration tests.
//!
//! A libtest `#[test]` runs on a worker thread while the harness owns the
//! main thread — and never services the AppKit dispatch queue, so the
//! first `Editor::new` would block forever (macos-editor-plan.md §3f).
//! The live tests are therefore `harness = false` binaries: their real
//! `main()` calls [`run_live_test`], which hands the main thread to
//! `NSApplication` and runs the test scenario on a worker thread — the
//! same split a CLAP host has (run loop on main, gui calls from the
//! engine control thread), and the same shape that verified items 3c/3d.
//!
//! The scenario thread ends the process: exit 0 on success, 101 on a
//! panic. A wall-clock watchdog backstops the whole run, because the
//! failure mode this exists for — a main-thread-dispatch deadlock — hangs
//! the scenario thread where no in-scenario assertion can see it. A test
//! that hangs forever is not a failing test, it is a stuck CI job.

use std::time::Duration;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

/// Pump `NSApplication` on the current (main) thread while `scenario`
/// runs on a worker thread. Never returns: the scenario or the watchdog
/// exits the process.
///
/// `budget` bounds the whole run, teardown included. It is not a
/// performance target — it is the line between "the runtime works" and
/// "the main-thread dispatch is wedged".
pub fn run_live_test(budget: Duration, scenario: impl FnOnce() + Send + 'static) -> ! {
    let mtm = MainThreadMarker::new()
        .expect("run_live_test must be called from the process main thread");
    let nsapp = NSApplication::sharedApplication(mtm);
    // A bare test binary has no activation policy; Regular lets the test
    // window become key, same as the hello example. (A plugin never does
    // this — the host application already has one.)
    nsapp.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    std::thread::Builder::new()
        .name("live-test-watchdog".to_string())
        .spawn(move || {
            std::thread::sleep(budget);
            eprintln!(
                "FAILED: live test still running after {budget:?} — the main-thread \
                 dispatch is wedged (the deadlock the editor_open watchdog exists for)"
            );
            std::process::exit(101);
        })
        .expect("could not spawn the live-test watchdog thread");

    std::thread::Builder::new()
        .name("live-test-scenario".to_string())
        .spawn(move || {
            // The panic (if any) has already printed its message and
            // location; this just turns it into the exit code.
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(scenario)) {
                Ok(()) => {
                    println!("ok — live scenario completed");
                    std::process::exit(0);
                }
                Err(_) => {
                    eprintln!("FAILED: live scenario panicked (see above)");
                    std::process::exit(101);
                }
            }
        })
        .expect("could not spawn the live-test scenario thread");

    // Hand the main thread to AppKit — in a plugin the host's run loop
    // does this. Never returns; the threads above end the process.
    nsapp.run();
    unreachable!("NSApplication::run returned");
}
