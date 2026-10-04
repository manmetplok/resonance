//! Tests for the shared repaint-scheduling decision
//! (`plugin_gui_core::repaint`), the logic both platform runtimes use to
//! turn egui's per-frame `repaint_delay` into either an immediate redraw
//! or a wake-up deadline.

use std::time::{Duration, Instant};

use plugin_gui_core::repaint::{plan_repaint, repaint_due, RepaintPlan};

#[test]
fn zero_delay_repaints_now() {
    // Duration::ZERO is what `Context::request_repaint()` reports —
    // the only case that should collapse to an immediate repaint.
    let now = Instant::now();
    assert_eq!(
        plan_repaint(now, Duration::ZERO, None),
        RepaintPlan::Now,
        "a real request_repaint() should repaint immediately"
    );
}

#[test]
fn now_supersedes_pending_deadline() {
    let now = Instant::now();
    let pending = Some(now + Duration::from_millis(500));
    assert_eq!(
        plan_repaint(now, Duration::ZERO, pending),
        RepaintPlan::Now
    );
}

#[test]
fn sixteen_and_thirtythree_ms_become_deadlines_not_now() {
    // PUX-04: editors call request_repaint_after(16ms|33ms) on every
    // frame to pace their own redraw. Collapsing that to `Now` repaints
    // at the monitor's refresh rate forever instead of the ~60/30 Hz
    // actually requested.
    let now = Instant::now();
    for ms in [1u64, 16, 33, 49, 50] {
        let delay = Duration::from_millis(ms);
        assert_eq!(
            plan_repaint(now, delay, None),
            RepaintPlan::At(now + delay),
            "{ms} ms should schedule a deadline, not repaint immediately"
        );
    }
}

#[test]
fn drums_style_100ms_request_schedules_a_deadline() {
    let now = Instant::now();
    let delay = Duration::from_millis(100);
    assert_eq!(plan_repaint(now, delay, None), RepaintPlan::At(now + delay));
}

#[test]
fn caret_blink_500ms_request_schedules_a_deadline() {
    let now = Instant::now();
    let delay = Duration::from_millis(500);
    assert_eq!(plan_repaint(now, delay, None), RepaintPlan::At(now + delay));
}

#[test]
fn earliest_deadline_wins_when_pending_is_sooner() {
    let now = Instant::now();
    let pending = now + Duration::from_millis(80);
    assert_eq!(
        plan_repaint(now, Duration::from_millis(500), Some(pending)),
        RepaintPlan::At(pending)
    );
}

#[test]
fn earliest_deadline_wins_when_request_is_sooner() {
    let now = Instant::now();
    let pending = now + Duration::from_millis(800);
    assert_eq!(
        plan_repaint(now, Duration::from_millis(100), Some(pending)),
        RepaintPlan::At(now + Duration::from_millis(100))
    );
}

#[test]
fn duration_max_means_idle_when_nothing_pending() {
    // egui's "no repaint needed" sentinel: Instant + Duration::MAX
    // overflows and must not schedule anything.
    let now = Instant::now();
    assert_eq!(plan_repaint(now, Duration::MAX, None), RepaintPlan::Idle);
}

#[test]
fn duration_max_keeps_a_pending_deadline() {
    let now = Instant::now();
    let pending = now + Duration::from_millis(120);
    assert_eq!(
        plan_repaint(now, Duration::MAX, Some(pending)),
        RepaintPlan::At(pending)
    );
}

#[test]
fn huge_finite_delay_saturates_like_never() {
    // Not Duration::MAX, but still beyond what Instant can represent:
    // must saturate to "never", not panic or wrap.
    let now = Instant::now();
    let huge = Duration::from_secs(u64::MAX / 2);
    assert_eq!(plan_repaint(now, huge, None), RepaintPlan::Idle);
    let pending = now + Duration::from_millis(60);
    assert_eq!(
        plan_repaint(now, huge, Some(pending)),
        RepaintPlan::At(pending)
    );
}

#[test]
fn repaint_due_semantics() {
    let now = Instant::now();
    assert!(!repaint_due(now, None));
    assert!(!repaint_due(now, Some(now + Duration::from_millis(1))));
    assert!(repaint_due(now, Some(now)), "a deadline at exactly now is due");
    assert!(repaint_due(now, Some(now - Duration::from_millis(1))));
}
