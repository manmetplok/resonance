//! Per-cycle DSP load metering for the realtime mix callback.
//!
//! The output process callback has a hard budget: `frames / sample_rate`
//! seconds per cycle (2.67 ms at 48 kHz / q128). Until now nothing
//! measured how much of that budget the mixer actually used, so "is the
//! engine the bottleneck?" could only be answered with external tools
//! (`pw-top` busy times). [`CycleLoadMeter`] times every mix call,
//! publishes a smoothed load + window peak into [`SharedState`] for
//! lock-free UI reads, counts over-budget cycles (a mix call that
//! outruns its cycle budget *is* an xrun), and hands a rate-limited
//! summary to the engine thread, which prints it alongside the graph's
//! own numbers:
//!
//! - default: one line per [`QUIET_REPORT_INTERVAL`], but only when the
//!   window contained something worth reporting (an over-budget cycle,
//!   a monitor-ring shortfall, or a peak above
//!   [`QUIET_PEAK_THRESHOLD`]) — a healthy idle session stays silent;
//! - `RESONANCE_AUDIO_STATS=1`: one line per
//!   [`VERBOSE_REPORT_INTERVAL`] unconditionally, for live load
//!   observation while diagnosing stutter.
//!
//! Monitor-ring shortfalls are counted by the mixer itself (a short
//! read while monitoring is a full quantum of dropped live input — an
//! audible stutter that no graph error counter catches, because every
//! stream still met its deadline); the meter only folds the counter
//! into its report line.
//!
//! There is no state-lock line: the callback reads the project through
//! the published render graph (`engine::render_graph`, code review
//! ARCH-02), a load that cannot miss, so a playing block always renders.
//!
//! RT-safety: `record` does arithmetic and relaxed atomic stores only.
//! The summary line is *not* formatted or printed on the audio thread:
//! the mix closure publishes the report into [`CycleReportSlot`] (a
//! seqlock of plain atomics on `SharedState`) and the engine control
//! loop formats + prints it on its next tick.
//!
//! See `tests/mixer/cycle_load.rs` for behaviour coverage.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::engine::SharedState;

/// Minimum gap between summary lines in the default (quiet) mode.
pub const QUIET_REPORT_INTERVAL: Duration = Duration::from_secs(10);

/// Minimum gap between summary lines with `RESONANCE_AUDIO_STATS=1`.
pub const VERBOSE_REPORT_INTERVAL: Duration = Duration::from_secs(2);

/// Window-peak load (fraction of the cycle budget) above which the
/// quiet mode reports even without an over-budget cycle — sustained
/// peaks this close to the budget mean any scheduling hiccup xruns.
pub const QUIET_PEAK_THRESHOLD: f32 = 0.75;

/// EMA coefficient for the smoothed load published to `SharedState`.
/// At 375 cycles/s (48 kHz / q128) this settles in roughly a third of
/// a second — steady enough for a UI meter, quick enough to track a
/// load spike.
pub const LOAD_EMA_ALPHA: f32 = 0.05;

/// One report-window summary, ready for formatting.
#[derive(Debug, Clone, PartialEq)]
pub struct CycleLoadReport {
    /// Mean load over the window (fraction of the cycle budget).
    pub avg: f32,
    /// Highest single-cycle load in the window.
    pub peak: f32,
    /// Cycles in the window whose mix call outran the cycle budget.
    pub overruns_window: u64,
    /// Lifetime over-budget cycles (mirrors `SharedState::dsp_overrun_cycles`).
    pub overruns_lifetime: u64,
    /// Monitor-ring shortfall cycles in the window (counted by the mixer).
    pub shortfalls_window: u64,
    /// Lifetime monitor-ring shortfall cycles.
    pub shortfalls_lifetime: u64,
}

/// The audio thread's hand-off of a [`CycleLoadReport`] to the engine
/// loop, which formats and prints it — so the realtime thread never
/// formats a `String` or writes to stderr.
///
/// A seqlock over plain atomics: `publish` bumps `seq` to odd, stores
/// the fields, bumps it to even; `take_new` returns nothing while a
/// publish is in flight and retries if one landed mid-read. Reports are
/// seconds apart and the reader polls every 16 ms, so both are
/// theoretical.
#[derive(Debug, Default)]
pub struct CycleReportSlot {
    seq: AtomicU64,
    avg_bits: AtomicU32,
    peak_bits: AtomicU32,
    overruns_window: AtomicU64,
    overruns_lifetime: AtomicU64,
    shortfalls_window: AtomicU64,
    shortfalls_lifetime: AtomicU64,
}

impl CycleReportSlot {
    /// Store `report` for the engine loop. Audio-thread side: atomics
    /// only.
    pub fn publish(&self, report: &CycleLoadReport) {
        self.seq.fetch_add(1, Ordering::Release);
        self.avg_bits.store(report.avg.to_bits(), Ordering::Relaxed);
        self.peak_bits.store(report.peak.to_bits(), Ordering::Relaxed);
        self.overruns_window
            .store(report.overruns_window, Ordering::Relaxed);
        self.overruns_lifetime
            .store(report.overruns_lifetime, Ordering::Relaxed);
        self.shortfalls_window
            .store(report.shortfalls_window, Ordering::Relaxed);
        self.shortfalls_lifetime
            .store(report.shortfalls_lifetime, Ordering::Relaxed);
        self.seq.fetch_add(1, Ordering::Release);
    }

    /// The report published since `last_seen` (the value this call
    /// wrote there last time; start at 0), or `None` if there is none.
    /// Engine-loop side.
    pub fn take_new(&self, last_seen: &mut u64) -> Option<CycleLoadReport> {
        loop {
            let before = self.seq.load(Ordering::Acquire);
            if before == *last_seen || before & 1 == 1 {
                return None;
            }
            let report = CycleLoadReport {
                avg: f32::from_bits(self.avg_bits.load(Ordering::Relaxed)),
                peak: f32::from_bits(self.peak_bits.load(Ordering::Relaxed)),
                overruns_window: self.overruns_window.load(Ordering::Relaxed),
                overruns_lifetime: self.overruns_lifetime.load(Ordering::Relaxed),
                shortfalls_window: self.shortfalls_window.load(Ordering::Relaxed),
                shortfalls_lifetime: self.shortfalls_lifetime.load(Ordering::Relaxed),
            };
            if self.seq.load(Ordering::Acquire) == before {
                *last_seen = before;
                return Some(report);
            }
        }
    }
}

/// The callback's one-shot "the backend asked for more frames than the
/// scratch holds" warning, handed to the engine loop to log (code
/// review ARCH-05 A5-2): the audio thread only stores two atomics.
///
/// Latches on the first oversize block — later ones are clamped
/// silently, exactly as before — and [`take_unreported`] hands it out
/// once.
///
/// [`take_unreported`]: OversizeBufferLatch::take_unreported
#[derive(Debug, Default)]
pub struct OversizeBufferLatch {
    /// Frames the backend requested; 0 until the first oversize block.
    requested: AtomicU64,
    /// Frames the pre-allocated scratch holds.
    scratch: AtomicU64,
    reported: AtomicBool,
}

impl OversizeBufferLatch {
    /// Record an oversize block. Audio-thread side: atomics only, and
    /// only the first call stores anything.
    pub fn record(&self, requested: usize, scratch: usize) {
        if self.requested.load(Ordering::Relaxed) != 0 {
            return;
        }
        self.scratch.store(scratch as u64, Ordering::Relaxed);
        let _ = self.requested.compare_exchange(
            0,
            requested as u64,
            Ordering::Release,
            Ordering::Relaxed,
        );
    }

    /// `(requested, scratch)` the first time it is called after an
    /// oversize block was recorded, `None` otherwise. Engine-loop side.
    pub fn take_unreported(&self) -> Option<(u64, u64)> {
        let requested = self.requested.load(Ordering::Acquire);
        if requested == 0 || self.reported.swap(true, Ordering::Relaxed) {
            return None;
        }
        Some((requested, self.scratch.load(Ordering::Relaxed)))
    }
}

/// Owned by the mix closure; not shared. All cross-thread publication
/// goes through `SharedState` atomics.
pub struct CycleLoadMeter {
    verbose: bool,
    interval: Duration,
    ema: f32,
    window_peak: f32,
    window_sum: f64,
    window_cycles: u64,
    window_overruns: u64,
    /// Lifetime shortfall count as of the last report, for the window delta.
    shortfalls_seen: u64,
    last_report: Option<Instant>,
}

impl CycleLoadMeter {
    pub fn new(verbose: bool) -> Self {
        Self {
            verbose,
            interval: if verbose {
                VERBOSE_REPORT_INTERVAL
            } else {
                QUIET_REPORT_INTERVAL
            },
            ema: 0.0,
            window_peak: 0.0,
            window_sum: 0.0,
            window_cycles: 0,
            window_overruns: 0,
            shortfalls_seen: 0,
            last_report: None,
        }
    }

    /// Record one mix call of `busy` wall time against a budget of
    /// `frames / sample_rate` seconds, publish the load atomics, and
    /// return a report when a summary line should be emitted.
    pub fn record(
        &mut self,
        now: Instant,
        busy: Duration,
        frames: usize,
        sample_rate: u32,
        shared: &SharedState,
    ) -> Option<CycleLoadReport> {
        if frames == 0 || sample_rate == 0 {
            return None;
        }
        let budget = frames as f32 / sample_rate as f32;
        let load = busy.as_secs_f32() / budget;

        self.ema += LOAD_EMA_ALPHA * (load - self.ema);
        self.window_peak = self.window_peak.max(load);
        self.window_sum += load as f64;
        self.window_cycles += 1;
        if load > 1.0 {
            self.window_overruns += 1;
            shared.dsp_overrun_cycles.fetch_add(1, Ordering::Relaxed);
        }
        shared
            .dsp_load_ema_bits
            .store(self.ema.to_bits(), Ordering::Relaxed);
        shared
            .dsp_load_peak_bits
            .store(self.window_peak.to_bits(), Ordering::Relaxed);

        let due = match self.last_report {
            // First cycle opens the window; nothing to summarize yet.
            None => {
                self.last_report = Some(now);
                return None;
            }
            Some(last) => now.saturating_duration_since(last) >= self.interval,
        };
        if !due {
            return None;
        }

        let shortfalls_lifetime = shared.monitor_shortfall_cycles.load(Ordering::Relaxed);
        let report = CycleLoadReport {
            avg: (self.window_sum / self.window_cycles as f64) as f32,
            peak: self.window_peak,
            overruns_window: self.window_overruns,
            overruns_lifetime: shared.dsp_overrun_cycles.load(Ordering::Relaxed),
            shortfalls_window: shortfalls_lifetime.saturating_sub(self.shortfalls_seen),
            shortfalls_lifetime,
        };
        self.last_report = Some(now);
        self.window_peak = 0.0;
        self.window_sum = 0.0;
        self.window_cycles = 0;
        self.window_overruns = 0;
        self.shortfalls_seen = shortfalls_lifetime;

        let noteworthy = report.overruns_window > 0
            || report.shortfalls_window > 0
            || report.peak >= QUIET_PEAK_THRESHOLD;
        (self.verbose || noteworthy).then_some(report)
    }
}

/// Format a load summary for stderr. Kept separate so tests can assert
/// on the exact wording without driving a real audio stream.
pub fn format_cycle_load_line(report: &CycleLoadReport) -> String {
    format!(
        "audio: dsp load avg {:.1}% peak {:.1}% | over-budget cycles {} (lifetime {}) | monitor shortfalls {} (lifetime {})",
        report.avg * 100.0,
        report.peak * 100.0,
        report.overruns_window,
        report.overruns_lifetime,
        report.shortfalls_window,
        report.shortfalls_lifetime,
    )
}
