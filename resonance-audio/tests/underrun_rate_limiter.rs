//! Regression tests for the cpal `StreamError::BufferUnderrun` rate
//! limiter wired into the output + input `err_fn` paths.
//!
//! Background: cpal 0.17 newly surfaces ALSA/JACK xruns through the
//! application's `err_fn` instead of writing them to cpal-internal
//! stderr. Without rate-limiting that floods the log on a busy desktop;
//! these tests pin the behaviour so a future refactor doesn't
//! accidentally reintroduce per-event spam.

use std::time::{Duration, Instant};

use resonance_audio::__test_support::{
    format_underrun_line, UnderrunRateLimiter, UNDERRUN_REPORT_INTERVAL,
};

const INTERVAL: Duration = Duration::from_secs(10);

#[test]
fn first_underrun_emits_immediately() {
    let limiter = UnderrunRateLimiter::new();
    let now = Instant::now();
    let report = limiter
        .record_with_interval(now, INTERVAL)
        .expect("first event always reports");
    assert_eq!(report.count, 1);
    assert_eq!(report.lifetime_total, 1);
}

#[test]
fn second_underrun_inside_interval_is_coalesced() {
    let limiter = UnderrunRateLimiter::new();
    let t0 = Instant::now();
    limiter.record_with_interval(t0, INTERVAL);
    // 1 second later — well inside the 10s interval.
    let t1 = t0 + Duration::from_secs(1);
    assert!(
        limiter.record_with_interval(t1, INTERVAL).is_none(),
        "events inside the report interval should be coalesced silently"
    );
}

#[test]
fn next_report_after_interval_carries_pending_count() {
    let limiter = UnderrunRateLimiter::new();
    let t0 = Instant::now();
    // Initial event emits.
    limiter.record_with_interval(t0, INTERVAL);
    // 9 more events coalesce.
    for i in 1..=9 {
        limiter.record_with_interval(t0 + Duration::from_millis(100 * i), INTERVAL);
    }
    // 10s later the next event should flush all 10 pending.
    let t_late = t0 + INTERVAL + Duration::from_millis(10);
    let report = limiter
        .record_with_interval(t_late, INTERVAL)
        .expect("interval elapsed → report fires");
    // 9 coalesced + the one that triggered the flush = 10 pending.
    assert_eq!(report.count, 10);
    // Lifetime total includes the very first reported event too.
    assert_eq!(report.lifetime_total, 11);
}

#[test]
fn lifetime_total_is_monotonic_across_reports() {
    let limiter = UnderrunRateLimiter::new();
    let t0 = Instant::now();
    let first = limiter.record_with_interval(t0, INTERVAL).unwrap();
    let second = limiter
        .record_with_interval(t0 + INTERVAL + Duration::from_millis(1), INTERVAL)
        .unwrap();
    let third = limiter
        .record_with_interval(t0 + INTERVAL * 3, INTERVAL)
        .unwrap();
    assert_eq!(first.lifetime_total, 1);
    assert_eq!(second.lifetime_total, 2);
    assert_eq!(third.lifetime_total, 3);
}

#[test]
fn coalesced_pending_resets_on_emit() {
    let limiter = UnderrunRateLimiter::new();
    let t0 = Instant::now();
    // First reported event.
    let first = limiter.record_with_interval(t0, INTERVAL).unwrap();
    assert_eq!(first.count, 1);
    // Three coalesced.
    for i in 1..=3 {
        limiter.record_with_interval(t0 + Duration::from_millis(50 * i), INTERVAL);
    }
    // 11s in — flushes pending (=4 incl. the trigger).
    let second = limiter
        .record_with_interval(t0 + INTERVAL + Duration::from_secs(1), INTERVAL)
        .unwrap();
    assert_eq!(second.count, 4);
    // Immediately afterwards the limiter should be back in "coalesce" mode.
    assert!(limiter
        .record_with_interval(
            t0 + INTERVAL + Duration::from_secs(1) + Duration::from_millis(1),
            INTERVAL
        )
        .is_none());
}

#[test]
fn default_report_interval_is_ten_seconds() {
    // Document the published constant — bumping it is a tunable
    // operator-visible change, so call it out in test if it ever moves.
    assert_eq!(UNDERRUN_REPORT_INTERVAL, Duration::from_secs(10));
}

#[test]
fn format_underrun_line_singular_form() {
    let limiter = UnderrunRateLimiter::new();
    let report = limiter
        .record_with_interval(Instant::now(), INTERVAL)
        .unwrap();
    let line = format_underrun_line("output", &report);
    // Singular wording avoids confusing the operator on the very first event.
    assert!(line.contains("output buffer underrun/overrun"), "got: {line}");
    assert!(line.contains("lifetime total: 1"), "got: {line}");
    assert!(!line.contains("underruns"), "should be singular: {line}");
}

#[test]
fn format_underrun_line_plural_form_includes_count_and_interval() {
    let limiter = UnderrunRateLimiter::new();
    let t0 = Instant::now();
    // Trigger first report, then accumulate 4 more before the next flush.
    limiter.record_with_interval(t0, INTERVAL);
    for i in 1..=4 {
        limiter.record_with_interval(t0 + Duration::from_millis(100 * i), INTERVAL);
    }
    let report = limiter
        .record_with_interval(t0 + INTERVAL + Duration::from_millis(1), INTERVAL)
        .unwrap();
    // 4 coalesced + the flush-trigger event = 5 pending.
    assert_eq!(report.count, 5);
    let line = format_underrun_line("input", &report);
    assert!(line.contains("input"), "got: {line}");
    assert!(line.contains("5 buffer underruns/overruns"), "got: {line}");
    assert!(line.contains("in the last 10s"), "got: {line}");
    assert!(line.contains("lifetime total: 6"), "got: {line}");
}
