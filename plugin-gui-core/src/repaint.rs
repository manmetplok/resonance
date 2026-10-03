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
//!
//! PUX-04: a 50 ms "repaint soon" threshold also swallowed the fixed
//! 16/33 ms intervals every editor requested on *every* frame
//! (`request_repaint_after`), which collapsed to [`RepaintPlan::Now`]
//! and repainted at the monitor's refresh rate forever — 60/144/240 Hz
//! for a visible editor, instead of the ~60/30 Hz the editor actually
//! asked for. egui reports exactly `Duration::ZERO` for a real
//! `Context::request_repaint()` (an immediate, "something changed"
//! request); any other value, however small, is a *paced* request from
//! `request_repaint_after` and is honoured as a deadline, never
//! collapsed to immediate.

use std::time::{Duration, Instant};

/// What a runtime should do about the `repaint_delay` egui reported for
/// the frame that just painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepaintPlan {
    /// Repaint as soon as frame pacing allows: `repaint_delay` was
    /// exactly zero, i.e. a real `Context::request_repaint()`.
    /// Supersedes any pending deadline: the frame it produces
    /// re-reports every request that is still live.
    Now,
    /// Repaint no later than this instant.
    At(Instant),
    /// Nothing requested and nothing pending: wait for input.
    Idle,
}

/// Fold the `repaint_delay` of a just-painted frame into the runtime's
/// pending repaint deadline.
///
/// - A zero delay (`Context::request_repaint()`) repaints immediately.
/// - Any other finite delay — including the 16/33 ms an editor requests
///   every frame to pace its own redraw — becomes a deadline; the
///   earliest of it and an already-pending one wins. This is what lets
///   an editor's `request_repaint_after(16ms)` actually cap it at ~60 Hz
///   instead of the monitor's refresh rate.
/// - An unschedulable delay (egui uses `Duration::MAX` for "no repaint
///   needed"; anything overflowing `Instant` counts) keeps whatever
///   deadline was already pending.
pub fn plan_repaint(
    now: Instant,
    repaint_after: Duration,
    pending: Option<Instant>,
) -> RepaintPlan {
    if repaint_after.is_zero() {
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
