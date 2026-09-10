//! Headless coverage for the teardown watchdog (`join_with_timeout`).
//!
//! `Editor::destroy` must never wedge the host thread — the CLAP host
//! runs editor teardown on the audio-engine control thread, and a plugin
//! `ui()` blocked in a modal run loop keeps the editor thread from ever
//! consuming `Quit`. The bounded join that guarantees this is pure std
//! (exposed via the crate's hidden `join_with_timeout` re-export), so it
//! is driven here with plain threads: no Wayland session, no display.

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use wayland_plugin_gui::join_with_timeout;

/// A thread that has already exited joins immediately and reports true —
/// the happy path must stay a real join (nothing detached, no waiting
/// out the timeout).
#[test]
fn finished_thread_joins_promptly() {
    let handle = thread::spawn(|| {});
    // Let it actually finish so the join is a pure reap.
    while !handle.is_finished() {
        thread::yield_now();
    }

    let start = Instant::now();
    assert!(
        join_with_timeout(handle, Duration::from_secs(10)),
        "a finished thread must join within the deadline"
    );
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "the happy path must not wait out the timeout (took {:?})",
        start.elapsed()
    );
}

/// A thread that exits while the watchdog is already waiting is still
/// reaped and reported true — completion mid-wait, not just
/// already-finished, takes the happy path.
#[test]
fn thread_exiting_mid_wait_joins() {
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let handle = thread::spawn(move || {
        // Parked until the timer below releases it.
        let _ = release_rx.recv();
    });
    // Release the "editor thread" shortly after the join has begun.
    let timer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        let _ = release_tx.send(());
    });

    let start = Instant::now();
    assert!(
        join_with_timeout(handle, Duration::from_secs(10)),
        "a thread that exits during the wait must be joined, not timed out"
    );
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "the join must return as soon as the thread exits (took {:?})",
        start.elapsed()
    );
    let _ = timer.join();
}

/// The wedge case: a thread that never exits (blocked the way a plugin
/// `ui()` blocks in a modal dialog) must NOT wedge the caller — the join
/// gives up at the deadline, reports false, and leaves the thread
/// detached.
#[test]
fn stuck_thread_times_out_instead_of_wedging() {
    // Keep the sender alive for the whole test so the thread stays
    // genuinely stuck for the duration of the bounded join.
    let (hold_tx, hold_rx) = mpsc::channel::<()>();
    let handle = thread::spawn(move || {
        // The stand-in for a plugin blocked inside ui(): waits forever
        // (until the test ends and `hold_tx` drops).
        let _ = hold_rx.recv();
    });

    let timeout = Duration::from_millis(250);
    let start = Instant::now();
    let joined = join_with_timeout(handle, timeout);
    let elapsed = start.elapsed();

    assert!(!joined, "a stuck thread must be reported as not joined");
    assert!(
        elapsed >= timeout,
        "the watchdog gave up before its deadline ({elapsed:?} < {timeout:?})"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the watchdog must return near its deadline, not wedge ({elapsed:?})"
    );

    // Unstick the leaked thread so the test binary exits cleanly.
    drop(hold_tx);
}
