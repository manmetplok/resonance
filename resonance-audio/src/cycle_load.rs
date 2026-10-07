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
//! Plugin lock misses are counted the same way (code review RT-07): a
//! live block that finds a plugin's lock held by another thread skips the
//! plugin — an effect passes dry, an instrument is silent for the block —
//! and [`record_plugin_lock_miss`] is the only trace of it. The process-wide
//! count is [`plugin_lock_misses`]; each `PluginSlot` keeps its own too.
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

use std::sync::atomic::{fence, AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::engine::SharedState;
use crate::mixer::render::slots::PassStats;

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

/// Live blocks, process-wide, that skipped a plugin because its lock was
/// held by another thread (see the module doc).
static PLUGIN_LOCK_MISSES: AtomicU64 = AtomicU64::new(0);

/// Count one plugin skipped on a busy lock. Audio-thread side: one relaxed
/// increment.
#[inline]
pub fn record_plugin_lock_miss() {
    PLUGIN_LOCK_MISSES.fetch_add(1, Ordering::Relaxed);
}

/// Plugin lock misses since the process started.
pub fn plugin_lock_misses() -> u64 {
    PLUGIN_LOCK_MISSES.load(Ordering::Relaxed)
}

/// An output callback that starts this long after its graph cycle began
/// counts as a late wakeup: with a 2.67 ms cycle there is little left for
/// the mix, and the graph may xrun before it finishes.
pub const LATE_WAKE_NS: u64 = 1_000_000;

/// Graph cycles that went by between two output callbacks, from the
/// driver clock: `prev_ticks` / `ticks` are `pw_time.ticks` at the two
/// callbacks, in units of `rate_num / rate_denom` seconds, and
/// `prev_frames` the engine frames (at `engine_rate`) the earlier one
/// rendered — the cycle length the clock should have moved by. A gap of
/// one and a half cycles or more counts the whole cycles beyond the
/// first. 0 when there is nothing to compare yet, the clock went
/// backwards, or it jumped by over a second (a stream (re)start or a
/// suspend, not a stutter). Arithmetic only — called on the RT thread.
pub fn missed_graph_cycles(
    prev_ticks: u64,
    ticks: u64,
    prev_frames: usize,
    rate_num: u32,
    rate_denom: u32,
    engine_rate: u32,
) -> u64 {
    if prev_ticks == 0 || prev_frames == 0 || rate_num == 0 || engine_rate == 0 {
        return 0;
    }
    let Some(delta) = ticks.checked_sub(prev_ticks) else {
        return 0;
    };
    // Ticks per second = denom / num.
    let ticks_per_second = rate_denom as u64 / rate_num as u64;
    if ticks_per_second == 0 || delta > ticks_per_second {
        return 0;
    }
    let cycle = (prev_frames as u64 * ticks_per_second / engine_rate as u64).max(1);
    if delta * 2 < cycle * 3 {
        return 0;
    }
    // Round to whole cycles: 2 cycles of clock is 1 missed.
    ((delta + cycle / 2) / cycle).saturating_sub(1)
}

/// One report-window summary, ready for formatting.
#[derive(Debug, Clone, PartialEq, Default)]
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
    /// Plugins skipped on a busy lock in the window
    /// ([`record_plugin_lock_miss`]).
    pub lock_misses_window: u64,
    /// Plugins skipped on a busy lock since the process started.
    pub lock_misses_lifetime: u64,
    /// Graph cycles that went by without this stream's callback in the
    /// window: the driver clock advanced more than one quantum between
    /// two callbacks (a late wakeup or a driver xrun). The mixer's own
    /// busy time cannot show these — it is only measured once the
    /// callback runs.
    pub graph_missed_window: u64,
    /// Lifetime graph cycles missed.
    pub graph_missed_lifetime: u64,
    /// Callbacks in the window that rendered nothing: no buffer to
    /// dequeue, no mapped data, or a misaligned one. Each is a quantum of
    /// silence the graph plays instead of the mix.
    pub output_dropped_window: u64,
    /// Lifetime output callbacks that rendered nothing.
    pub output_dropped_lifetime: u64,
    /// The native output stream's longest wakeup delay (graph cycle start
    /// to callback) and longest whole callback in the window, in µs; 0 on
    /// a backend that does not report them.
    pub wake_max_us: u32,
    pub callback_max_us: u32,
    /// Callbacks that woke over [`LATE_WAKE_NS`] after their cycle began.
    pub late_wakes_window: u64,
    pub late_wakes_lifetime: u64,
    /// The track pass's parallel rendering over the window
    /// (realtime-multithreading.md §6). All zero when nothing was
    /// recorded, and then left out of the line.
    pub pool: PoolReport,
}

/// The render pool's share of a [`CycleLoadReport`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PoolReport {
    /// Most threads a block's jobs spread over (the audio thread
    /// included); 0 = no track pass recorded.
    pub threads: u32,
    /// The longest single track job in the window, and whose it was. The
    /// heaviest job bounds a block however many threads there are, so
    /// this names the track to freeze.
    pub critical_us: u32,
    pub critical_track: Option<u64>,
    /// Summed job time / (job-phase wall time × threads): 1.0 = every
    /// thread busy until the join.
    pub efficiency: f32,
    /// Mean time per cycle the audio thread spun at the join with no job
    /// left. High with low efficiency means one job dominates.
    pub join_wait_us: f32,
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
    lock_misses_window: AtomicU64,
    lock_misses_lifetime: AtomicU64,
    graph_missed_window: AtomicU64,
    graph_missed_lifetime: AtomicU64,
    output_dropped_window: AtomicU64,
    output_dropped_lifetime: AtomicU64,
    wake_max_us: AtomicU32,
    callback_max_us: AtomicU32,
    late_wakes_window: AtomicU64,
    late_wakes_lifetime: AtomicU64,
    pool_threads: AtomicU32,
    pool_critical_us: AtomicU32,
    /// `track id + 1`; 0 = none.
    pool_critical_track: AtomicU64,
    pool_efficiency_bits: AtomicU32,
    pool_join_wait_bits: AtomicU32,
}

impl CycleReportSlot {
    /// Store `report` for the engine loop. Audio-thread side: atomics
    /// only.
    pub fn publish(&self, report: &CycleLoadReport) {
        // The standard seqlock writer (code review RT-16): a Release RMW
        // only orders what came *before* it, so on a weakly ordered CPU
        // (aarch64, the macOS target) the field stores below could become
        // visible before `seq` turns odd, and a reader would accept a torn
        // report. The Release fence keeps every later store after the odd
        // count.
        self.seq.fetch_add(1, Ordering::Relaxed);
        fence(Ordering::Release);
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
        self.lock_misses_window
            .store(report.lock_misses_window, Ordering::Relaxed);
        self.lock_misses_lifetime
            .store(report.lock_misses_lifetime, Ordering::Relaxed);
        self.graph_missed_window
            .store(report.graph_missed_window, Ordering::Relaxed);
        self.graph_missed_lifetime
            .store(report.graph_missed_lifetime, Ordering::Relaxed);
        self.output_dropped_window
            .store(report.output_dropped_window, Ordering::Relaxed);
        self.output_dropped_lifetime
            .store(report.output_dropped_lifetime, Ordering::Relaxed);
        self.wake_max_us.store(report.wake_max_us, Ordering::Relaxed);
        self.callback_max_us
            .store(report.callback_max_us, Ordering::Relaxed);
        self.late_wakes_window
            .store(report.late_wakes_window, Ordering::Relaxed);
        self.late_wakes_lifetime
            .store(report.late_wakes_lifetime, Ordering::Relaxed);
        let pool = &report.pool;
        self.pool_threads.store(pool.threads, Ordering::Relaxed);
        self.pool_critical_us
            .store(pool.critical_us, Ordering::Relaxed);
        self.pool_critical_track
            .store(pool.critical_track.map_or(0, |t| t + 1), Ordering::Relaxed);
        self.pool_efficiency_bits
            .store(pool.efficiency.to_bits(), Ordering::Relaxed);
        self.pool_join_wait_bits
            .store(pool.join_wait_us.to_bits(), Ordering::Relaxed);
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
                lock_misses_window: self.lock_misses_window.load(Ordering::Relaxed),
                lock_misses_lifetime: self.lock_misses_lifetime.load(Ordering::Relaxed),
                graph_missed_window: self.graph_missed_window.load(Ordering::Relaxed),
                graph_missed_lifetime: self.graph_missed_lifetime.load(Ordering::Relaxed),
                output_dropped_window: self.output_dropped_window.load(Ordering::Relaxed),
                output_dropped_lifetime: self.output_dropped_lifetime.load(Ordering::Relaxed),
                wake_max_us: self.wake_max_us.load(Ordering::Relaxed),
                callback_max_us: self.callback_max_us.load(Ordering::Relaxed),
                late_wakes_window: self.late_wakes_window.load(Ordering::Relaxed),
                late_wakes_lifetime: self.late_wakes_lifetime.load(Ordering::Relaxed),
                pool: PoolReport {
                    threads: self.pool_threads.load(Ordering::Relaxed),
                    critical_us: self.pool_critical_us.load(Ordering::Relaxed),
                    critical_track: self
                        .pool_critical_track
                        .load(Ordering::Relaxed)
                        .checked_sub(1),
                    efficiency: f32::from_bits(self.pool_efficiency_bits.load(Ordering::Relaxed)),
                    join_wait_us: f32::from_bits(self.pool_join_wait_bits.load(Ordering::Relaxed)),
                },
            };
            // The reader half (RT-16): the Relaxed field loads above may
            // not be reordered after the re-check, which an Acquire load
            // alone does not forbid; the Acquire fence does, pairing with
            // the writer's fence.
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == before {
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
    /// Lifetime plugin lock-miss count as of the last report.
    lock_misses_seen: u64,
    /// Lifetime missed-graph-cycle and dropped-output counts as of the
    /// last report.
    graph_missed_seen: u64,
    output_dropped_seen: u64,
    late_wakes_seen: u64,
    /// Where the lifetime lock-miss count comes from:
    /// [`plugin_lock_misses`], unless a test substitutes its own
    /// ([`Self::with_lock_miss_source`]) — the real count is process-wide,
    /// and other tests in the same binary move it.
    lock_misses: fn() -> u64,
    last_report: Option<Instant>,
    /// Track-pass stats over the window ([`Self::record_pass`]).
    pass_threads: usize,
    pass_critical_ns: u64,
    pass_critical_track: Option<u64>,
    pass_jobs_ns: u64,
    /// Job-phase wall time × threads.
    pass_capacity_ns: u64,
    pass_join_wait_ns: u64,
    pass_cycles: u64,
}

#[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
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
            lock_misses_seen: plugin_lock_misses(),
            graph_missed_seen: 0,
            output_dropped_seen: 0,
            late_wakes_seen: 0,
            lock_misses: plugin_lock_misses,
            last_report: None,
            pass_threads: 0,
            pass_critical_ns: 0,
            pass_critical_track: None,
            pass_jobs_ns: 0,
            pass_capacity_ns: 0,
            pass_join_wait_ns: 0,
            pass_cycles: 0,
        }
    }

    /// Read the lifetime plugin lock-miss count from `source` instead of
    /// the process-wide [`plugin_lock_misses`] (tests).
    #[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
    pub fn with_lock_miss_source(mut self, source: fn() -> u64) -> Self {
        self.lock_misses = source;
        self.lock_misses_seen = source();
        self
    }

    /// Fold one callback's track-pass stats into the window. Call before
    /// [`Self::record`] for the same cycle. Arithmetic only.
    pub fn record_pass(&mut self, stats: &PassStats) {
        if stats.threads == 0 {
            return;
        }
        self.pass_threads = self.pass_threads.max(stats.threads);
        if stats.critical_ns > self.pass_critical_ns {
            self.pass_critical_ns = stats.critical_ns;
            self.pass_critical_track = stats.critical_track;
        }
        self.pass_jobs_ns += stats.jobs_ns;
        self.pass_capacity_ns += stats.wall_ns * stats.threads as u64;
        self.pass_join_wait_ns += stats.join_wait_ns;
        self.pass_cycles += 1;
    }

    fn take_pool_report(&mut self) -> PoolReport {
        let report = PoolReport {
            threads: self.pass_threads as u32,
            critical_us: (self.pass_critical_ns / 1_000).min(u32::MAX as u64) as u32,
            critical_track: self.pass_critical_track,
            efficiency: if self.pass_capacity_ns == 0 {
                0.0
            } else {
                (self.pass_jobs_ns as f64 / self.pass_capacity_ns as f64) as f32
            },
            join_wait_us: if self.pass_cycles == 0 {
                0.0
            } else {
                (self.pass_join_wait_ns as f64 / self.pass_cycles as f64 / 1_000.0) as f32
            },
        };
        self.pass_threads = 0;
        self.pass_critical_ns = 0;
        self.pass_critical_track = None;
        self.pass_jobs_ns = 0;
        self.pass_capacity_ns = 0;
        self.pass_join_wait_ns = 0;
        self.pass_cycles = 0;
        report
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
        let lock_misses_lifetime = (self.lock_misses)();
        let graph_missed_lifetime = shared.graph_missed_cycles.load(Ordering::Relaxed);
        let output_dropped_lifetime = shared.output_dropped_cycles.load(Ordering::Relaxed);
        let late_wakes_lifetime = shared.output_late_wakes.load(Ordering::Relaxed);
        let ns_to_us = |ns: u64| (ns / 1_000).min(u32::MAX as u64) as u32;
        let report = CycleLoadReport {
            avg: (self.window_sum / self.window_cycles as f64) as f32,
            peak: self.window_peak,
            overruns_window: self.window_overruns,
            overruns_lifetime: shared.dsp_overrun_cycles.load(Ordering::Relaxed),
            shortfalls_window: shortfalls_lifetime.saturating_sub(self.shortfalls_seen),
            shortfalls_lifetime,
            lock_misses_window: lock_misses_lifetime.saturating_sub(self.lock_misses_seen),
            lock_misses_lifetime,
            graph_missed_window: graph_missed_lifetime.saturating_sub(self.graph_missed_seen),
            graph_missed_lifetime,
            output_dropped_window: output_dropped_lifetime.saturating_sub(self.output_dropped_seen),
            output_dropped_lifetime,
            wake_max_us: ns_to_us(shared.output_wake_max_ns.swap(0, Ordering::Relaxed)),
            callback_max_us: ns_to_us(shared.output_callback_max_ns.swap(0, Ordering::Relaxed)),
            late_wakes_window: late_wakes_lifetime.saturating_sub(self.late_wakes_seen),
            late_wakes_lifetime,
            pool: self.take_pool_report(),
        };
        self.last_report = Some(now);
        self.window_peak = 0.0;
        self.window_sum = 0.0;
        self.window_cycles = 0;
        self.window_overruns = 0;
        self.shortfalls_seen = shortfalls_lifetime;
        self.lock_misses_seen = lock_misses_lifetime;
        self.graph_missed_seen = graph_missed_lifetime;
        self.output_dropped_seen = output_dropped_lifetime;
        self.late_wakes_seen = late_wakes_lifetime;

        let noteworthy = report.overruns_window > 0
            || report.shortfalls_window > 0
            || report.lock_misses_window > 0
            || report.graph_missed_window > 0
            || report.output_dropped_window > 0
            || report.late_wakes_window > 0
            || report.peak >= QUIET_PEAK_THRESHOLD;
        (self.verbose || noteworthy).then_some(report)
    }
}

/// Format a load summary for stderr. Kept separate so tests can assert
/// on the exact wording without driving a real audio stream.
pub fn format_cycle_load_line(report: &CycleLoadReport) -> String {
    let mut line = format!(
        "audio: dsp load avg {:.1}% peak {:.1}% | over-budget cycles {} (lifetime {}) | monitor shortfalls {} (lifetime {})",
        report.avg * 100.0,
        report.peak * 100.0,
        report.overruns_window,
        report.overruns_lifetime,
        report.shortfalls_window,
        report.shortfalls_lifetime,
    );
    if report.lock_misses_lifetime > 0 {
        line.push_str(&format!(
            " | plugin lock misses {} (lifetime {})",
            report.lock_misses_window, report.lock_misses_lifetime,
        ));
    }
    if report.graph_missed_lifetime > 0 {
        line.push_str(&format!(
            " | missed graph cycles {} (lifetime {})",
            report.graph_missed_window, report.graph_missed_lifetime,
        ));
    }
    if report.output_dropped_lifetime > 0 {
        line.push_str(&format!(
            " | dropped output cycles {} (lifetime {})",
            report.output_dropped_window, report.output_dropped_lifetime,
        ));
    }
    if report.wake_max_us > 0 || report.late_wakes_lifetime > 0 {
        line.push_str(&format!(
            " | stream wake max {} µs, callback max {} µs, late wakes {} (lifetime {})",
            report.wake_max_us,
            report.callback_max_us,
            report.late_wakes_window,
            report.late_wakes_lifetime,
        ));
    }
    let pool = &report.pool;
    if pool.threads > 0 {
        let track = pool
            .critical_track
            .map_or_else(|| "-".to_owned(), |t| t.to_string());
        line.push_str(&format!(
            " | render {} threads, critical {} µs (track {track}), efficiency {:.0}%, join wait {:.1} µs",
            pool.threads,
            pool.critical_us,
            pool.efficiency * 100.0,
            pool.join_wait_us,
        ));
    }
    line
}
