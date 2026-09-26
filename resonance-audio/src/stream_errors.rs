//! Helpers for the cpal stream `err_fn` callbacks.
//!
//! cpal 0.17 added [`cpal::StreamError::BufferUnderrun`] and now reports
//! ALSA / JACK buffer underruns + overruns through the application's
//! `err_fn` instead of writing them to stderr inside cpal itself
//! (see the cpal 0.17.0 changelog). On a busy desktop with PipeWire /
//! ALSA-compat that easily fires multiple times a second under normal
//! UI load, so naively logging every event spams the log even
//! though the stream itself recovers silently.
//!
//! [`UnderrunRateLimiter`] coalesces those events into one summary line
//! every [`UNDERRUN_REPORT_INTERVAL`] (and emits the first one
//! immediately so a real problem is still visible right away). Other
//! `StreamError` variants (`DeviceNotAvailable`, `StreamInvalidated`,
//! `BackendSpecific`) are rare and load-bearing — those still go
//! through `tracing::error!` directly.
//!
//! See `tests/mixer/underrun_rate_limiter.rs` for behaviour coverage.
//!
//! ALSA itself also recovers silently (`PCM.try_recover(silent=true)`)
//! since cpal 0.17, so we do *not* need to forward to cpal — the
//! recovery has already happened by the time `err_fn` fires.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
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
        self.record_count_with_interval(now, 1, interval)
    }

    /// Register `count` underruns at once — what the engine loop drains
    /// from a [`StreamErrorLatch`] per tick. `count == 0` records nothing.
    pub fn record_count(&self, now: Instant, count: u64) -> Option<UnderrunReport> {
        self.record_count_with_interval(now, count, UNDERRUN_REPORT_INTERVAL)
    }

    fn record_count_with_interval(
        &self,
        now: Instant,
        count: u64,
        interval: Duration,
    ) -> Option<UnderrunReport> {
        if count == 0 {
            return None;
        }
        let mut state = self.inner.lock().expect("poisoned underrun limiter");
        state.pending += count;
        state.total += count;

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

/// A cpal stream's error callback, reduced to atomics (code review
/// FU-H6b). cpal runs `err_fn` on its ALSA worker — the audio thread —
/// so it only counts here; the engine loop drains the counts, rate-limits
/// the underruns through an [`UnderrunRateLimiter`] and formats + logs.
/// A backend-specific error's description is copied, truncated to
/// [`STREAM_ERROR_TEXT_MAX`] bytes, into a fixed atomic byte buffer — no
/// allocation — and turned back into text on the engine side (FU-A4c).
#[derive(Debug)]
pub struct StreamErrorLatch {
    underruns: AtomicU64,
    errors: AtomicU64,
    /// The most recent non-underrun error's kind ([`Self::kind_name`]).
    last_error_kind: AtomicU8,
    /// The latest backend-specific description, `text_len` bytes of it.
    text: [AtomicU8; STREAM_ERROR_TEXT_MAX],
    text_len: AtomicU8,
    /// Exclusive access to `text` / `text_len` for whichever side sets it
    /// first; the other skips (the writer drops that text, the reader
    /// takes it next tick). Never waited on.
    text_busy: AtomicBool,
}

/// Bytes of a backend-specific stream error's description the
/// [`StreamErrorLatch`] keeps.
pub const STREAM_ERROR_TEXT_MAX: usize = 160;

impl Default for StreamErrorLatch {
    fn default() -> Self {
        Self {
            underruns: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            last_error_kind: AtomicU8::new(0),
            text: std::array::from_fn(|_| AtomicU8::new(0)),
            text_len: AtomicU8::new(0),
            text_busy: AtomicBool::new(false),
        }
    }
}

impl StreamErrorLatch {
    /// Audio-thread side: count one error. Atomics only.
    pub fn record(&self, err: &cpal::StreamError) {
        let kind = match err {
            cpal::StreamError::BufferUnderrun => {
                self.underruns.fetch_add(1, Ordering::Relaxed);
                return;
            }
            cpal::StreamError::DeviceNotAvailable => 1,
            cpal::StreamError::StreamInvalidated => 2,
            cpal::StreamError::BackendSpecific { err } => {
                self.store_text(&err.description);
                3
            }
        };
        self.last_error_kind.store(kind, Ordering::Relaxed);
        self.errors.fetch_add(1, Ordering::Release);
    }

    /// Engine-loop side: underruns since the last call.
    pub fn take_underruns(&self) -> u64 {
        self.underruns.swap(0, Ordering::Relaxed)
    }

    /// Engine-loop side: `(count, kind of the latest)` of the other
    /// errors since the last call, or `None`.
    pub fn take_errors(&self) -> Option<(u64, &'static str)> {
        let count = self.errors.swap(0, Ordering::Acquire);
        (count > 0).then(|| {
            (
                count,
                Self::kind_name(self.last_error_kind.load(Ordering::Relaxed)),
            )
        })
    }

    /// Audio-thread side: keep `text` (cut on a char boundary at
    /// [`STREAM_ERROR_TEXT_MAX`] bytes). Atomics only; dropped if the
    /// engine side is reading at that moment.
    fn store_text(&self, text: &str) {
        if self.text_busy.swap(true, Ordering::Acquire) {
            return;
        }
        let mut len = text.len().min(STREAM_ERROR_TEXT_MAX);
        while !text.is_char_boundary(len) {
            len -= 1;
        }
        for (slot, &byte) in self.text.iter().zip(&text.as_bytes()[..len]) {
            slot.store(byte, Ordering::Relaxed);
        }
        self.text_len.store(len as u8, Ordering::Relaxed);
        self.text_busy.store(false, Ordering::Release);
    }

    /// Engine-loop side: the latest backend-specific error description,
    /// once (cleared by the call). `None` when there is none, or when the
    /// audio side is writing one right now (it is picked up next call).
    pub fn take_error_text(&self) -> Option<String> {
        if self.text_busy.swap(true, Ordering::Acquire) {
            return None;
        }
        let len = self.text_len.swap(0, Ordering::Relaxed) as usize;
        let bytes: Vec<u8> = self.text[..len]
            .iter()
            .map(|b| b.load(Ordering::Relaxed))
            .collect();
        self.text_busy.store(false, Ordering::Release);
        (len > 0).then(|| String::from_utf8_lossy(&bytes).into_owned())
    }

    fn kind_name(kind: u8) -> &'static str {
        match kind {
            1 => "device no longer available",
            2 => "stream configuration invalidated",
            _ => "backend-specific error",
        }
    }
}
