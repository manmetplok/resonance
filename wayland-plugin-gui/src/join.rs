//! Bounded thread join — the teardown watchdog.
//!
//! `std::thread::JoinHandle` has no timed join, so the bound is enforced
//! the way `AudioEngine::shutdown` enforces its deadline: the handle is
//! moved into a companion watchdog thread that performs the (possibly
//! wedging) join, and the caller waits for the watchdog's completion
//! signal under a wall-clock timeout. On the happy path the join
//! completes promptly and the watchdog itself is reaped — nothing leaks.
//! On timeout the watchdog, and the wedged thread it holds, are left
//! detached, by design: leaking one thread beats wedging the caller.
//! The caller here is the CLAP host destroying an editor on the
//! audio-engine control thread — an unbounded join there takes the whole
//! `AudioCommand` queue down with it (transport, volume, shutdown).

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

/// Join `handle`, waiting at most `timeout`.
///
/// Returns `true` when the thread exited within the deadline (the join
/// completed, the watchdog was reaped, nothing leaked). Returns `false`
/// on timeout: the thread is left running, detached, together with the
/// watchdog thread still waiting on it.
pub fn join_with_timeout(handle: JoinHandle<()>, timeout: Duration) -> bool {
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let watchdog = std::thread::Builder::new()
        .name("wpg-join-watchdog".to_string())
        .spawn(move || {
            let _ = handle.join();
            // A closed receiver means the caller timed out and moved
            // on; there is nobody left to signal, which is fine.
            let _ = done_tx.send(());
        });
    let Ok(watchdog) = watchdog else {
        // The watchdog could not be spawned; its closure — and the
        // handle inside it — were dropped, detaching the thread
        // unjoined. Treat it as the timeout path: the caller must not
        // block, whatever happens.
        return false;
    };
    match done_rx.recv_timeout(timeout) {
        Ok(()) => {
            // The join completed; reap the watchdog too so the happy
            // path leaks nothing.
            let _ = watchdog.join();
            true
        }
        Err(_) => false,
    }
}

/// How [`await_startup`] ended.
#[derive(Debug)]
pub enum Startup<E> {
    /// The thread reported ready; here is its handle to keep.
    Ready(JoinHandle<()>),
    /// The thread reported a setup error; it has been reaped (bounded).
    Failed(E),
    /// The thread went away without reporting anything.
    Disconnected,
    /// No report within the deadline: `abort` was called and the thread
    /// reaped, or detached if it did not exit within `reap_timeout`.
    TimedOut,
}

/// Wait for a freshly spawned editor thread's startup report — the
/// creation-side twin of [`join_with_timeout`] (PLG-10).
///
/// The CLAP host creates editors on the same audio-engine control thread
/// it destroys them on, with the instance lock held, so the wait for the
/// thread's first configure must be bounded just like teardown: a
/// compositor that never configures the window (hung, restarting, a
/// kiosk shell) would otherwise wedge that thread forever. On timeout,
/// `abort` asks the thread to quit (the pre-configure loop watches for
/// it) and the thread is reaped under `reap_timeout`.
pub fn await_startup<E>(
    ready_rx: &mpsc::Receiver<Result<(), E>>,
    thread: JoinHandle<()>,
    timeout: Duration,
    abort: impl FnOnce(),
    reap_timeout: Duration,
) -> Startup<E> {
    match ready_rx.recv_timeout(timeout) {
        Ok(Ok(())) => Startup::Ready(thread),
        Ok(Err(err)) => {
            // The thread hit a setup error and is exiting; reap it,
            // bounded all the same.
            let _ = join_with_timeout(thread, reap_timeout);
            Startup::Failed(err)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = join_with_timeout(thread, reap_timeout);
            Startup::Disconnected
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            abort();
            let _ = join_with_timeout(thread, reap_timeout);
            Startup::TimedOut
        }
    }
}
