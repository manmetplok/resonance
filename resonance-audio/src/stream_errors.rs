//! Helpers for the cpal stream `err_fn` callbacks.
//!
//! cpal 0.17 added [`cpal::StreamError::BufferUnderrun`] and now reports
//! ALSA / JACK buffer underruns + overruns through the application's
//! `err_fn` instead of writing them to stderr inside cpal itself
//! (see the cpal 0.17.0 changelog). On a busy desktop with PipeWire /
//! ALSA-compat that easily fires multiple times a second under normal
//! UI load, so naively `eprintln!`-ing every event spams the log even
//! though the stream itself recovers silently.
//!
//! [`UnderrunRateLimiter`] coalesces those events into one summary line
//! every [`UNDERRUN_REPORT_INTERVAL`] (and emits the first one
//! immediately so a real problem is still visible right away). Other
//! `StreamError` variants (`DeviceNotAvailable`, `StreamInvalidated`,
//! `BackendSpecific`) are rare and load-bearing — those still go
//! through `eprintln!` directly.
//!
//! See `tests/underrun_rate_limiter.rs` for behaviour coverage.
//!
//! ALSA itself also recovers silently (`PCM.try_recover(silent=true)`)
//! since cpal 0.17, so we do *not* need to forward to cpal — the
//! recovery has already happened by the time `err_fn` fires.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Minimum gap between underrun summary lines. Long enough that a
/// once-in-a-while xrun under load doesn't spam the log; short enough
/// that a sustained problem still shows up quickly.
pub const UNDERRUN_REPORT_INTERVAL: Duration = Duration::from_secs(10);

/// Rate-limiter for `StreamError::BufferUnderrun` events. Records every
/// occurrence and tells the caller when it's time to emit a summary
/// line.
#[derive(Debug, Default)]
pub struct UnderrunRateLimiter {
    inner: Mutex<UnderrunState>,
}

#[derive(Debug, Default)]
struct UnderrunState {
    /// Number of underruns since the last emitted summary line.
    pending: u64,
    /// Total underruns over the lifetime of this limiter — included in
    /// every summary so operators can spot a slow leak even after the
    /// rate has stabilised.
    total: u64,
    /// Timestamp of the last summary line we emitted, or `None` if
    /// we've never emitted one. The first event always emits
    /// immediately so a sudden burst is visible right away.
    last_report: Option<Instant>,
}

/// What `UnderrunRateLimiter::record` decided to do with the event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnderrunReport {
    /// Number of underruns this report covers (always ≥ 1).
    pub count: u64,
    /// Lifetime total including this batch.
    pub lifetime_total: u64,
}

impl UnderrunRateLimiter {
    /// Build a fresh rate-limiter. `pending` starts at 0; the first
    /// `record()` call will produce a summary immediately.
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(UnderrunState {
                pending: 0,
                total: 0,
                last_report: None,
            }),
        }
    }

    /// Register one underrun and decide whether to emit a summary now.
    /// Returns `Some(report)` if the caller should log a line; `None`
    /// if the event was coalesced into the running counter.
    pub fn record(&self, now: Instant) -> Option<UnderrunReport> {
        self.record_with_interval(now, UNDERRUN_REPORT_INTERVAL)
    }

    /// Same as [`record`] but with a caller-supplied interval. Only
    /// used by the tests so they don't have to sleep for real seconds.
    pub fn record_with_interval(
        &self,
        now: Instant,
        interval: Duration,
    ) -> Option<UnderrunReport> {
        let mut state = self.inner.lock().expect("poisoned underrun limiter");
        state.pending += 1;
        state.total += 1;

        let should_emit = match state.last_report {
            None => true,
            Some(last) => now.saturating_duration_since(last) >= interval,
        };

        if should_emit {
            let report = UnderrunReport {
                count: state.pending,
                lifetime_total: state.total,
            };
            state.pending = 0;
            state.last_report = Some(now);
            Some(report)
        } else {
            None
        }
    }
}

/// Format a buffer-underrun report for stderr. Kept separate so tests
/// can assert on the exact wording without driving an actual cpal
/// stream.
pub fn format_underrun_line(label: &str, report: &UnderrunReport) -> String {
    if report.count == 1 {
        format!(
            "audio: {} buffer underrun/overrun (lifetime total: {})",
            label, report.lifetime_total
        )
    } else {
        format!(
            "audio: {} {} buffer underruns/overruns in the last {}s (lifetime total: {})",
            label,
            report.count,
            UNDERRUN_REPORT_INTERVAL.as_secs(),
            report.lifetime_total,
        )
    }
}
