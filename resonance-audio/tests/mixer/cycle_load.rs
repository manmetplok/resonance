//! Behaviour coverage for the per-cycle DSP load meter
//! (`src/cycle_load.rs`): load math against the cycle budget,
//! over-budget counting, quiet-mode report gating, monitor-shortfall
//! window deltas, the per-map lock-miss attribution (code review
//! ARCH-02 A2-1), the audio→engine report hand-off, and the stderr
//! line format.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use resonance_audio::test_support::{
    format_cycle_load_line, CycleLoadMeter, CycleLoadReport, CycleReportSlot, MixAudioHarness,
    SharedState, StateMap, QUIET_PEAK_THRESHOLD, QUIET_REPORT_INTERVAL, STATE_MAP_COUNT,
    VERBOSE_REPORT_INTERVAL,
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
fn quiet_mode_reports_per_map_lock_misses_as_window_delta() {
    let shared = SharedState::default();
    let mut meter = CycleLoadMeter::new(false);
    let start = Instant::now();
    let cycles = (QUIET_REPORT_INTERVAL.as_secs_f64() / budget().as_secs_f64()) as usize + 50;
    shared.lock_misses.record(StateMap::Plugins);
    shared.lock_misses.record(StateMap::Plugins);
    shared.lock_misses.record(StateMap::Clips);
    // A miss alone (no render skip: e.g. the stopped branch's monitor
    // pass) is noteworthy — it is a dropout the graph never reports.
    let report = drive(&mut meter, &shared, start, cycles, budget() / 100).expect("report");
    assert_eq!(report.lock_skips_window, 0);
    assert_eq!(report.lock_misses_window, [0, 0, 0, 1, 0, 2]);
    assert_eq!(report.lock_misses_lifetime, [0, 0, 0, 1, 0, 2]);

    // Next window: one more on tracks only; the others read zero.
    shared.lock_misses.record(StateMap::Tracks);
    let start2 = start + budget() * (cycles as u32 + 1);
    let report = drive(&mut meter, &shared, start2, cycles, budget() / 100).expect("report");
    assert_eq!(report.lock_misses_window, [1, 0, 0, 0, 0, 0]);
    assert_eq!(report.lock_misses_lifetime, [1, 0, 0, 1, 0, 2]);

    // And silent again once nothing moves.
    let start3 = start2 + budget() * (cycles as u32 + 1);
    assert_eq!(drive(&mut meter, &shared, start3, cycles, budget() / 100), None);
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
        lock_skips_window: 5,
        lock_skips_lifetime: 6,
        lock_misses_window: [1, 2, 3, 4, 5, 6],
        lock_misses_lifetime: [7, 8, 9, 10, 11, 12],
    };
    slot.publish(&report);
    assert_eq!(slot.take_new(&mut seen), Some(report.clone()));
    assert_eq!(slot.take_new(&mut seen), None, "same report is not re-printed");

    let mut next = report.clone();
    next.lock_misses_window = [0; STATE_MAP_COUNT];
    slot.publish(&next);
    assert_eq!(slot.take_new(&mut seen), Some(next));
    assert_eq!(slot.take_new(&mut seen), None);
}

/// A minimal playing project: one track, one clip, so the playing
/// branch takes all five map read guards.
fn playing_harness() -> MixAudioHarness {
    let track = Track::new(1, "clips".into());
    track.set_output(TrackOutput::Master);
    let clip = AudioClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        source: ClipSource::Memory(vec![0.1; 64 * FRAMES * 2]),
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
fn callback_attributes_a_contended_block_to_the_map_that_missed() {
    let mut h = playing_harness();

    // Uncontended: the block renders, nothing is counted.
    assert!(h.render().iter().any(|&s| s != 0.0));
    assert_eq!(h.shared().lock_misses.snapshot(), [0; STATE_MAP_COUNT]);
    assert_eq!(h.shared().render_skip_cycles.load(Ordering::Relaxed), 0);

    // A write-held clips map (the UI-edit shape).
    assert!(h.render_lock_contended().iter().all(|&s| s == 0.0));
    assert_eq!(h.shared().render_skip_cycles.load(Ordering::Relaxed), 1);
    assert_eq!(h.shared().lock_misses.snapshot(), [0, 0, 0, 1, 0, 0]);
    assert_eq!(h.shared().lock_misses.get(StateMap::Clips), 1);

    // The ARCH-02 shape: a read guard held on a worker thread with a
    // writer queued behind it. Only that map's counter moves, and the
    // block is still skipped as a whole.
    for (i, map) in StateMap::ALL.iter().enumerate() {
        if *map == StateMap::Master {
            // The master lock is taken by the master pass, after the
            // arrangement rendered; a miss there drops the master FX
            // for the block, not the render.
            continue;
        }
        let before = h.shared().lock_misses.snapshot();
        let skips_before = h.shared().render_skip_cycles.load(Ordering::Relaxed);
        assert!(h.render_with_queued_writer(*map).iter().all(|&s| s == 0.0));
        let after = h.shared().lock_misses.snapshot();
        let mut expected = before;
        expected[i] += 1;
        assert_eq!(after, expected, "only {} should have missed", map.name());
        assert_eq!(
            h.shared().render_skip_cycles.load(Ordering::Relaxed),
            skips_before + 1
        );
    }

    // Master: the render goes through (audio out), the master-FX pass
    // counts its miss.
    let before = h.shared().lock_misses.get(StateMap::Master);
    let skips_before = h.shared().render_skip_cycles.load(Ordering::Relaxed);
    assert!(h.render_with_queued_writer(StateMap::Master).iter().any(|&s| s != 0.0));
    assert_eq!(h.shared().lock_misses.get(StateMap::Master), before + 1);
    assert_eq!(h.shared().render_skip_cycles.load(Ordering::Relaxed), skips_before);

    // Back to uncontended: the counters hold, nothing new.
    let before = h.shared().lock_misses.snapshot();
    assert!(h.render().iter().any(|&s| s != 0.0));
    assert_eq!(h.shared().lock_misses.snapshot(), before);
}

#[test]
fn stopped_branch_counts_its_misses_without_a_render_skip() {
    let mut h = playing_harness();
    h.shared().playing.store(false, Ordering::Relaxed);
    h.render_with_queued_writer(StateMap::Plugins);
    assert_eq!(h.shared().lock_misses.snapshot(), [0, 0, 0, 0, 0, 1]);
    assert_eq!(h.shared().render_skip_cycles.load(Ordering::Relaxed), 0);
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
        lock_misses_window: [0, 0, 0, 1, 0, 0],
        lock_misses_lifetime: [2, 0, 0, 4, 0, 1],
    });
    assert_eq!(
        line,
        "audio: dsp load avg 3.2% peak 41.0% | over-budget cycles 2 (lifetime 15) | monitor shortfalls 0 (lifetime 3) | render lock-skips 1 (lifetime 6) | lock misses tracks 0 busses 0 master 0 clips 1 midi-clips 0 plugins 0 (lifetime 2/0/0/4/0/1)"
    );
}
