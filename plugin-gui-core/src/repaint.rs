//! Pure scheduling for egui repaint requests, shared by the platform
//! runtimes (`wayland-plugin-gui`, `cocoa-plugin-gui`).
//!
//! After every painted frame egui reports a `repaint_delay`: how soon it
//! wants the next frame (`Duration::MAX` when nothing was requested).
//! The runtimes used to honour only delays under 50 ms and silently drop
//! the rest, which froze anything animating below ~20 Hz — the drums
//! editor's deliberate 10 Hz meter, egui's ~500 ms text-caret blink.
//! The scheduling decision lives here so both runtimes translate a
//! finite future delay into a deadline they wake on, identically.

use std::time::{Duration, Instant};

/// Delays under this repaint immediately (within the runtime's normal
/// frame pacing) instead of via a deadline — the historical behaviour
/// for "soon" requests, kept unchanged.
pub const REPAINT_SOON: Duration = Duration::from_millis(50);

/// What a runtime should do about the `repaint_delay` egui reported for
/// the frame that just painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepaintPlan {
    /// Repaint as soon as frame pacing allows (delay under
    /// [`REPAINT_SOON`]). Supersedes any pending deadline: the frame it
    /// produces re-reports every request that is still live.
    Now,
    /// Repaint no later than this instant.
    At(Instant),
    /// Nothing requested and nothing pending: wait for input.
    Idle,
}

/// Fold the `repaint_delay` of a just-painted frame into the runtime's
/// pending repaint deadline.
///
/// - Delays under [`REPAINT_SOON`] repaint immediately.
/// - Finite longer delays become a deadline; the earliest of it and an
///   already-pending one wins.
/// - An unschedulable delay (egui uses `Duration::MAX` for "no repaint
///   needed"; anything overflowing `Instant` counts) keeps whatever
///   deadline was already pending.
pub fn plan_repaint(
    now: Instant,
    repaint_after: Duration,
    pending: Option<Instant>,
) -> RepaintPlan {
    if repaint_after < REPAINT_SOON {
        return RepaintPlan::Now;
    }
    match (pending, now.checked_add(repaint_after)) {
        (Some(p), Some(r)) => RepaintPlan::At(p.min(r)),
        (Some(p), None) => RepaintPlan::At(p),
        (None, Some(r)) => RepaintPlan::At(r),
        (None, None) => RepaintPlan::Idle,
    }
}

/// Whether a stored repaint deadline has come due at `now`.
pub fn repaint_due(now: Instant, pending: Option<Instant>) -> bool {
    pending.is_some_and(|at| at <= now)
}
