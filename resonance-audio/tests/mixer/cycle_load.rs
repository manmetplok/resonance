//! Behaviour coverage for the per-cycle DSP load meter
//! (`src/cycle_load.rs`): load math against the cycle budget,
//! over-budget counting, quiet-mode report gating, monitor-shortfall
//! window deltas, the audio→engine report hand-off, the stderr line
//! format, and that a playing block always renders (the render graph
//! cannot miss — code review ARCH-02).

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use resonance_audio::test_support::{
    format_cycle_load_line, CycleLoadMeter, CycleLoadReport, CycleReportSlot, MixAudioHarness,
    PassStats, PoolReport,
    SharedState, QUIET_PEAK_THRESHOLD, QUIET_REPORT_INTERVAL, VERBOSE_REPORT_INTERVAL,
};
use resonance_audio::types::*;

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
    let mut meter = CycleLoadMeter::new(false).with_lock_miss_source(no_misses);
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
    let mut meter = CycleLoadMeter::new(false).with_lock_miss_source(no_misses);
    let start = Instant::now();
    drive(&mut meter, &shared, start, 3, budget() * 2);
    assert_eq!(shared.dsp_overrun_cycles.load(Ordering::Relaxed), 3);
}

/// The process-wide lock-miss count moves with every other test in this
/// binary; the meters here read a quiet one instead.
fn no_misses() -> u64 {
    0
}

#[test]
fn quiet_mode_stays_silent_when_healthy() {
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(false).with_lock_miss_source(no_misses);
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
    let mut meter = CycleLoadMeter::new(false).with_lock_miss_source(no_misses);
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
    let mut meter = CycleLoadMeter::new(false).with_lock_miss_source(no_misses);
    let busy = budget().mul_f32(QUIET_PEAK_THRESHOLD + 0.05);
    let report = drive(&mut meter, &shared, start, cycles, busy).expect("report");
    assert_eq!(report.overruns_window, 0);
    assert!(report.peak >= QUIET_PEAK_THRESHOLD);
}

#[test]
fn quiet_mode_reports_monitor_shortfalls_as_window_delta() {
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(false).with_lock_miss_source(no_misses);
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

/// A plugin skipped on a busy lock is reason enough for a quiet-mode line
/// (code review RT-07), reported as the window's delta.
#[test]
fn quiet_mode_reports_plugin_lock_misses_as_window_delta() {
    static MISSES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    fn misses() -> u64 {
        MISSES.load(Ordering::Relaxed)
    }
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(false).with_lock_miss_source(misses);
    let start = Instant::now();
    let cycles = (QUIET_REPORT_INTERVAL.as_secs_f64() / budget().as_secs_f64()) as usize + 50;
    MISSES.store(4, Ordering::Relaxed);
    let report = drive(&mut meter, &shared, start, cycles, budget() / 100).expect("report");
    assert_eq!(report.lock_misses_window, 4);
    assert_eq!(report.lock_misses_lifetime, 4);

    let start2 = start + budget() * (cycles as u32 + 1);
    let report = drive(&mut meter, &shared, start2, cycles, budget() / 100);
    assert_eq!(report, None, "no new misses: silent again");
}

#[test]
fn report_slot_hands_each_report_to_the_engine_loop_once() {
    let slot = CycleReportSlot::default();
    let mut seen = 0u64;
    assert_eq!(slot.take_new(&mut seen), None, "nothing published yet");

    let report = CycleLoadReport {
        avg: 0.25,
        peak: 0.9,
        overruns_window: 1,
        overruns_lifetime: 2,
        shortfalls_window: 3,
        shortfalls_lifetime: 4,
        lock_misses_window: 5,
        lock_misses_lifetime: 6,
        pool: PoolReport {
            threads: 8,
            critical_us: 1_234,
            critical_track: Some(7),
            efficiency: 0.8,
            join_wait_us: 12.5,
        },
    };
    slot.publish(&report);
    assert_eq!(slot.take_new(&mut seen), Some(report.clone()));
    assert_eq!(slot.take_new(&mut seen), None, "same report is not re-printed");

    let mut next = report.clone();
    next.shortfalls_window = 0;
    slot.publish(&next);
    assert_eq!(slot.take_new(&mut seen), Some(next));
    assert_eq!(slot.take_new(&mut seen), None);
}

/// RT-16: the seqlock never hands out a torn report while a writer
/// publishes flat out. Every field of report `i` is derived from `i`, so
/// a mix of two publishes is detectable. (x86 is strongly ordered enough
/// to pass even without the fences; on aarch64 this is the race they
/// close.)
#[test]
fn report_slot_never_hands_out_a_torn_report() {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn report(i: u64) -> CycleLoadReport {
        CycleLoadReport {
            avg: i as f32,
            peak: i as f32,
            overruns_window: i,
            overruns_lifetime: i,
            shortfalls_window: i,
            shortfalls_lifetime: i,
            lock_misses_window: i,
            lock_misses_lifetime: i,
            pool: PoolReport {
                threads: i as u32,
                critical_us: i as u32,
                critical_track: Some(i),
                efficiency: i as f32,
                join_wait_us: i as f32,
            },
        }
    }

    let slot = Arc::new(CycleReportSlot::default());
    let done = Arc::new(AtomicBool::new(false));
    let writer = {
        let (slot, done) = (Arc::clone(&slot), Arc::clone(&done));
        std::thread::spawn(move || {
            let mut i = 1u64;
            while !done.load(Ordering::Relaxed) {
                slot.publish(&report(i % 1_000_000));
                i += 1;
            }
        })
    };
    let mut seen = 0u64;
    let mut read = 0;
    let deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < deadline {
        if let Some(r) = slot.take_new(&mut seen) {
            assert_eq!(r, report(r.overruns_window), "torn report");
            read += 1;
        }
    }
    done.store(true, Ordering::Relaxed);
    writer.join().unwrap();
    assert!(read > 0, "the reader saw reports");
}

/// A minimal playing project: one track, one clip.
fn playing_harness() -> MixAudioHarness {
    let mut track = Track::new(1, "clips".into());
    track.set_output(TrackOutput::Master);
    let clip = AudioClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        source: ClipSource::memory(vec![0.1; 64 * FRAMES * 2]),
        name: "c1".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::Linear,
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: WarpAlgorithm::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    };
    let mut tempo = TempoMap::default();
    tempo.rebuild_bar_table(RATE);
    let h = MixAudioHarness::new(
        vec![track],
        Vec::new(),
        vec![clip],
        Vec::new(),
        Vec::new(),
        tempo,
        FRAMES,
        2,
        RATE,
        true,
    );
    h.shared().playing.store(true, Ordering::Relaxed);
    h
}

#[test]
fn a_playing_block_always_renders() {
    // Every map the playing branch reads is in the render graph (ARCH-02
    // B-5 moved the last one, the clips): a load that cannot miss, and
    // since B-6 there is no skip path — every block renders and advances.
    let mut h = playing_harness();
    for block in 1..=4u64 {
        assert!(h.render().iter().any(|&s| s != 0.0));
        assert_eq!(h.shared().playhead.load(Ordering::Relaxed), block * FRAMES as u64);
    }
}

/// ARCH-05 A5-2: a host buffer larger than the scratch is clamped, and
/// the callback only latches the sizes into `SharedState` — the engine
/// loop takes them once and logs them. Later oversize blocks change
/// nothing.
#[test]
fn oversize_host_buffer_is_latched_for_the_engine_loop_once() {
    let mut h = playing_harness();
    assert_eq!(h.shared().oversize_buffer.take_unreported(), None);

    h.render();
    assert_eq!(h.shared().oversize_buffer.take_unreported(), None);

    h.set_host_buffer_frames(FRAMES * 2);
    let out = h.render();
    // Clamped: only the scratch's worth rendered, the tail stays silent.
    assert!(out[..FRAMES * 2].iter().any(|&s| s != 0.0));
    assert!(out[FRAMES * 2..].iter().all(|&s| s == 0.0));
    assert_eq!(
        h.shared().oversize_buffer.take_unreported(),
        Some((FRAMES as u64 * 2, FRAMES as u64))
    );
    assert_eq!(h.shared().oversize_buffer.take_unreported(), None);

    h.set_host_buffer_frames(FRAMES * 4);
    h.render();
    assert_eq!(h.shared().oversize_buffer.take_unreported(), None);
}

#[test]
fn verbose_mode_reports_unconditionally_on_its_interval() {
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(true).with_lock_miss_source(no_misses);
    let start = Instant::now();
    let cycles = (VERBOSE_REPORT_INTERVAL.as_secs_f64() / budget().as_secs_f64()) as usize + 20;
    let report = drive(&mut meter, &shared, start, cycles, budget() / 100).expect("report");
    assert!(report.avg < 0.05);
    assert_eq!(report.overruns_window, 0);
}

#[test]
fn degenerate_cycles_are_ignored() {
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(true).with_lock_miss_source(no_misses);
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
        lock_misses_window: 0,
        lock_misses_lifetime: 0,
        pool: PoolReport::default(),
    });
    assert_eq!(
        line,
        "audio: dsp load avg 3.2% peak 41.0% | over-budget cycles 2 (lifetime 15) | monitor shortfalls 0 (lifetime 3)"
    );
}

/// A plugin the render skipped on a busy lock passed dry audio (or none)
/// for a block (code review RT-07); once that has happened at all, the
/// line carries the count.
#[test]
fn line_reports_plugin_lock_misses_once_there_are_any() {
    let line = format_cycle_load_line(&CycleLoadReport {
        lock_misses_window: 2,
        lock_misses_lifetime: 9,
        ..CycleLoadReport::default()
    });
    assert!(
        line.ends_with("| monitor shortfalls 0 (lifetime 0) | plugin lock misses 2 (lifetime 9)"),
        "{line}"
    );
}

/// With the render pool's stats in the window, the line names the
/// heaviest job's track — the one to freeze — and how well the threads
/// were used (realtime-multithreading.md §6).
#[test]
fn line_names_the_critical_track_when_the_pool_reported() {
    let line = format_cycle_load_line(&CycleLoadReport {
        avg: 0.5,
        peak: 0.9,
        pool: PoolReport {
            threads: 8,
            critical_us: 1_900,
            critical_track: Some(12),
            efficiency: 0.625,
            join_wait_us: 310.25,
        },
        ..CycleLoadReport::default()
    });
    assert!(
        line.ends_with(
            "| render 8 threads, critical 1900 µs (track 12), efficiency 62%, join wait 310.2 µs"
        ),
        "{line}"
    );
}

/// The meter keeps the window's heaviest job and relates summed job time
/// to the capacity the threads offered.
#[test]
fn pass_stats_fold_into_the_windows_report() {
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(true).with_lock_miss_source(no_misses);
    let t0 = std::time::Instant::now();
    meter.record(t0, budget() / 2, FRAMES, RATE, &shared);
    for (critical_ns, track) in [(400_000u64, 3u64), (900_000, 5), (100_000, 3)] {
        meter.record_pass(&PassStats {
            jobs_ns: 1_000_000,
            wall_ns: 500_000,
            critical_ns,
            critical_track: Some(track),
            join_wait_ns: 50_000,
            threads: 4,
        });
    }
    let report = meter
        .record(t0 + std::time::Duration::from_secs(5), budget() / 2, FRAMES, RATE, &shared)
        .expect("verbose meter reports once the interval passed");
    assert_eq!(report.pool.threads, 4);
    assert_eq!(report.pool.critical_us, 900);
    assert_eq!(report.pool.critical_track, Some(5));
    assert_eq!(report.pool.efficiency, 0.5, "1 ms of work per 0.5 ms × 4 threads");
    assert_eq!(report.pool.join_wait_us, 50.0);

    meter.record_pass(&PassStats::default());
    let next = meter
        .record(t0 + std::time::Duration::from_secs(10), budget() / 2, FRAMES, RATE, &shared)
        .expect("reports again");
    assert_eq!(next.pool, PoolReport::default(), "the window's pool stats reset");
}
