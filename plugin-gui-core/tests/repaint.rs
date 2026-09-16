//! Tests for the shared repaint-scheduling decision
//! (`plugin_gui_core::repaint`), the logic both platform runtimes use to
//! turn egui's per-frame `repaint_delay` into either an immediate redraw
//! or a wake-up deadline.

use std::time::{Duration, Instant};

use plugin_gui_core::repaint::{plan_repaint, repaint_due, RepaintPlan, REPAINT_SOON};

#[test]
fn short_delays_repaint_now() {
    let now = Instant::now();
    for ms in [0u64, 1, 16, 49] {
        assert_eq!(
            plan_repaint(now, Duration::from_millis(ms), None),
            RepaintPlan::Now,
            "{ms} ms should repaint immediately"
        );
    }
}

#[test]
fn now_supersedes_pending_deadline() {
    let now = Instant::now();
    let pending = Some(now + Duration::from_millis(500));
    assert_eq!(
        plan_repaint(now, Duration::from_millis(10), pending),
        RepaintPlan::Now
    );
}

#[test]
fn threshold_delay_becomes_deadline_not_dropped() {
    // The original bug: exactly-50 ms (and anything longer) was discarded.
    let now = Instant::now();
    assert_eq!(
        plan_repaint(now, REPAINT_SOON, None),
        RepaintPlan::At(now + REPAINT_SOON)
    );
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
