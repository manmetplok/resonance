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

use wayland_plugin_gui::{await_startup, join_with_timeout, Startup};

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

// ---------------------------------------------------------------------------
// The startup handshake (`await_startup`, PLG-10)
// ---------------------------------------------------------------------------

/// A thread that reports ready is handed back, promptly and joinable.
#[test]
fn startup_ready_returns_the_thread() {
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
    let handle = thread::spawn(move || {
        let _ = ready_tx.send(Ok(()));
    });
    match await_startup(
        &ready_rx,
        handle,
        Duration::from_secs(10),
        || panic!("a ready thread must not be aborted"),
        Duration::from_secs(10),
    ) {
        Startup::Ready(handle) => handle.join().expect("editor thread"),
        other => panic!("expected Ready, got {other:?}"),
    }
}

/// A setup error is passed through and the failing thread reaped.
#[test]
fn startup_failure_is_reported() {
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
    let handle = thread::spawn(move || {
        let _ = ready_tx.send(Err("no compositor".to_string()));
    });
    let outcome = await_startup(
        &ready_rx,
        handle,
        Duration::from_secs(10),
        || {},
        Duration::from_secs(10),
    );
    match outcome {
        Startup::Failed(err) => assert_eq!(err, "no compositor"),
        other => panic!("expected Failed, got {other:?}"),
    }
}

/// The wedge case: the editor thread waits for a first configure that
/// never comes. `Editor::new` used to block on it forever — on the host's
/// engine thread, with the instance lock held. The wait must give up at
/// its deadline, ask the thread to quit, and reap it.
#[test]
fn startup_that_never_reports_times_out_and_aborts_the_thread() {
    let (_ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
    let (quit_tx, quit_rx) = mpsc::channel::<()>();
    // The stand-in for the pre-configure loop: never ready, but it does
    // watch for Quit.
    let handle = thread::spawn(move || {
        let _ = quit_rx.recv();
    });
    let timeout = Duration::from_millis(250);
    let start = Instant::now();
    let outcome = await_startup(
        &ready_rx,
        handle,
        timeout,
        move || {
            let _ = quit_tx.send(());
        },
        Duration::from_secs(10),
    );
    let elapsed = start.elapsed();
    assert!(matches!(outcome, Startup::TimedOut), "got {outcome:?}");
    assert!(elapsed >= timeout, "gave up early ({elapsed:?})");
    assert!(
        elapsed < Duration::from_secs(5),
        "the abort must reap the thread promptly ({elapsed:?})"
    );
}

/// A thread that ignores the abort too (stuck in the Wayland roundtrip)
/// is detached after the reap deadline: the caller still returns.
#[test]
fn startup_stuck_past_the_abort_is_detached() {
    let (_ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
    let (hold_tx, hold_rx) = mpsc::channel::<()>();
    let handle = thread::spawn(move || {
        let _ = hold_rx.recv();
    });
    let start = Instant::now();
    let outcome = await_startup(
        &ready_rx,
        handle,
        Duration::from_millis(100),
        || {},
        Duration::from_millis(100),
    );
    let elapsed = start.elapsed();
    assert!(matches!(outcome, Startup::TimedOut), "got {outcome:?}");
    assert!(elapsed < Duration::from_secs(5), "wedged ({elapsed:?})");
    drop(hold_tx);
}
