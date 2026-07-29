//! Behaviour coverage for the per-cycle DSP load meter
//! (`src/cycle_load.rs`): load math against the cycle budget,
//! over-budget counting, quiet-mode report gating, monitor-shortfall
//! window deltas, and the stderr line format.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use resonance_audio::__test_support::{
    format_cycle_load_line, CycleLoadMeter, CycleLoadReport, SharedState, QUIET_PEAK_THRESHOLD,
    QUIET_REPORT_INTERVAL, VERBOSE_REPORT_INTERVAL,
};

/// 128 frames @ 48 kHz — the pinned engine cycle. Budget = 2.667 ms.
const FRAMES: usize = 128;
const RATE: u32 = 48_000;

fn budget() -> Duration {
    Duration::from_secs_f64(FRAMES as f64 / RATE as f64)
}

/// Drive `n` cycles of identical `busy` time, one budget apart,
/// returning the last report the meter emitted (if any).
fn drive(
    meter: &mut CycleLoadMeter,
    shared: &SharedState,
    start: Instant,
    n: usize,
    busy: Duration,
) -> Option<CycleLoadReport> {
    let mut last = None;
    for i in 0..n {
        let now = start + budget() * (i as u32 + 1);
        if let Some(r) = meter.record(now, busy, FRAMES, RATE, shared) {
            last = Some(r);
        }
    }
    last
}

#[test]
fn publishes_load_atomics_every_cycle() {
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(false);
    let start = Instant::now();
    // Half the budget per cycle -> load 0.5.
    drive(&mut meter, &shared, start, 10, budget() / 2);
    let ema = f32::from_bits(shared.dsp_load_ema_bits.load(Ordering::Relaxed));
    let peak = f32::from_bits(shared.dsp_load_peak_bits.load(Ordering::Relaxed));
    assert!(ema > 0.0 && ema < 0.5, "EMA converging toward 0.5: {ema}");
    assert!((peak - 0.5).abs() < 0.01, "window peak ~0.5: {peak}");
    assert_eq!(shared.dsp_overrun_cycles.load(Ordering::Relaxed), 0);
}

#[test]
fn counts_over_budget_cycles() {
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(false);
    let start = Instant::now();
    drive(&mut meter, &shared, start, 3, budget() * 2);
    assert_eq!(shared.dsp_overrun_cycles.load(Ordering::Relaxed), 3);
}

#[test]
fn quiet_mode_stays_silent_when_healthy() {
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(false);
    let start = Instant::now();
    // Low load, no shortfalls: well past the quiet interval with no report.
    let cycles = (QUIET_REPORT_INTERVAL.as_secs_f64() / budget().as_secs_f64()) as usize + 50;
    let report = drive(&mut meter, &shared, start, cycles, budget() / 100);
    assert_eq!(report, None);
}

#[test]
fn quiet_mode_reports_over_budget_and_near_budget_peaks() {
    // An over-budget cycle forces a report.
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(false);
    let start = Instant::now();
    let cycles = (QUIET_REPORT_INTERVAL.as_secs_f64() / budget().as_secs_f64()) as usize + 50;
    let report = drive(&mut meter, &shared, start, cycles, budget() * 2).expect("report");
    assert!(report.overruns_window > 0);
    // Lifetime as of the report covers every cycle up to that point;
    // the shared counter keeps counting cycles after the last report.
    assert!(report.overruns_lifetime >= report.overruns_window);
    assert_eq!(shared.dsp_overrun_cycles.load(Ordering::Relaxed), cycles as u64);

    // A peak at the threshold (but under budget) also forces one.
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(false);
    let busy = budget().mul_f32(QUIET_PEAK_THRESHOLD + 0.05);
    let report = drive(&mut meter, &shared, start, cycles, busy).expect("report");
    assert_eq!(report.overruns_window, 0);
    assert!(report.peak >= QUIET_PEAK_THRESHOLD);
}

#[test]
fn quiet_mode_reports_monitor_shortfalls_as_window_delta() {
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(false);
    let start = Instant::now();
    let cycles = (QUIET_REPORT_INTERVAL.as_secs_f64() / budget().as_secs_f64()) as usize + 50;
    shared.monitor_shortfall_cycles.store(7, Ordering::Relaxed);
    let report = drive(&mut meter, &shared, start, cycles, budget() / 100).expect("report");
    assert_eq!(report.shortfalls_window, 7);
    assert_eq!(report.shortfalls_lifetime, 7);

    // Next window: no new shortfalls -> silent again.
    let start2 = start + budget() * (cycles as u32 + 1);
    let report = drive(&mut meter, &shared, start2, cycles, budget() / 100);
    assert_eq!(report, None);
}

#[test]
fn quiet_mode_reports_render_lock_skips_as_window_delta() {
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(false);
    let start = Instant::now();
    let cycles = (QUIET_REPORT_INTERVAL.as_secs_f64() / budget().as_secs_f64()) as usize + 50;
    shared.render_skip_cycles.store(4, Ordering::Relaxed);
    let report = drive(&mut meter, &shared, start, cycles, budget() / 100).expect("report");
    assert_eq!(report.lock_skips_window, 4);
    assert_eq!(report.lock_skips_lifetime, 4);

    // Next window: no new skips -> silent again.
    let start2 = start + budget() * (cycles as u32 + 1);
    let report = drive(&mut meter, &shared, start2, cycles, budget() / 100);
    assert_eq!(report, None);
}

#[test]
fn verbose_mode_reports_unconditionally_on_its_interval() {
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(true);
    let start = Instant::now();
    let cycles = (VERBOSE_REPORT_INTERVAL.as_secs_f64() / budget().as_secs_f64()) as usize + 20;
    let report = drive(&mut meter, &shared, start, cycles, budget() / 100).expect("report");
    assert!(report.avg < 0.05);
    assert_eq!(report.overruns_window, 0);
}

#[test]
fn degenerate_cycles_are_ignored() {
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(true);
    let now = Instant::now();
    assert_eq!(meter.record(now, budget(), 0, RATE, &shared), None);
    assert_eq!(meter.record(now, budget(), FRAMES, 0, &shared), None);
    assert_eq!(shared.dsp_load_ema_bits.load(Ordering::Relaxed), 0);
}

#[test]
fn line_format_is_stable() {
    let line = format_cycle_load_line(&CycleLoadReport {
        avg: 0.032,
        peak: 0.41,
        overruns_window: 2,
        overruns_lifetime: 15,
        shortfalls_window: 0,
        shortfalls_lifetime: 3,
        lock_skips_window: 1,
        lock_skips_lifetime: 6,
    });
    assert_eq!(
        line,
        "audio: dsp load avg 3.2% peak 41.0% | over-budget cycles 2 (lifetime 15) | monitor shortfalls 0 (lifetime 3) | render lock-skips 1 (lifetime 6)"
    );
}
