//! Panic supervision for detached worker threads (offline renders and
//! queued clip imports).
//!
//! Every offline render runs on a spawned worker whose *expected*
//! failures already flow back to the app as a terminal `AudioEvent`
//! (`BounceError` / `ExportError` / `FreezeError` / `StemExportError` /
//! `MixMeasureError`). But those worker bodies drive the full offline
//! mixer — third-party CLAP plugin `process()` included — and a panic
//! anywhere in that stack used to unwind straight out of the thread
//! body: the thread died with NO terminal event, so the app's progress
//! modal never resolved and a control client's `job_wait` reported
//! "running" until its wait cap. [`run_supervised`] closes that hole:
//! it contains the unwind and hands a formatted message to an
//! `on_panic` callback, which each spawn site uses to emit the *same*
//! error event it already emits for expected failures.
//!
//! The clip-import worker pool ([`ImportQueue`]) has the sibling
//! problem — a panicking job killed a pool worker permanently — and
//! reuses the same wrapper around each job.
//!
//! [`ImportQueue`]: crate::engine::ImportQueue

use std::panic::{catch_unwind, AssertUnwindSafe};

/// Best-effort human-readable text out of a panic payload: the `&str`
/// and `String` payloads `panic!` produces are passed through, anything
/// else (`std::panic::panic_any` with an arbitrary type) becomes a
/// placeholder.
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic".to_owned()
    }
}

/// Run `body`, containing any panic. On panic, `on_panic` receives
/// `"<name> worker panicked: <payload>"`; spawn sites use it to emit
/// their path's terminal error event so whatever is waiting on the
/// worker (progress modal, control client) resolves instead of hanging
/// forever. On success `on_panic` is not called — the body has already
/// reported its own outcome.
///
/// `AssertUnwindSafe` is sound here: the engine state a worker captures
/// is shared behind locks either way, so a panicking render leaves it
/// no more inconsistent than the same panic did before supervision —
/// and the terminal error event tells the app the run is over. RAII
/// guards created inside `body` (`OfflineRenderGuard`, lock guards)
/// drop during the unwind, *before* `on_panic` runs, so the panic path
/// releases the offline-render slot exactly like the error path does.
///
/// The default panic hook still runs before the unwind is caught, so
/// the panic location and backtrace keep landing on stderr.
pub fn run_supervised<F, P>(name: &str, body: F, on_panic: P)
where
    F: FnOnce(),
    P: FnOnce(String),
{
    if let Err(payload) = catch_unwind(AssertUnwindSafe(body)) {
        on_panic(format!(
            "{name} worker panicked: {}",
            panic_message(payload.as_ref())
        ));
    }
}
