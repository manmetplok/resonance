//! Behaviour coverage for the per-cycle DSP load meter
//! (`src/cycle_load.rs`): load math against the cycle budget,
//! over-budget counting, quiet-mode report gating, monitor-shortfall
//! window deltas, the (now empty) per-map lock-miss table (code review
//! ARCH-02 A2-1; B-5 moved the last locked map into the render graph),
//! the audio→engine report hand-off, and the stderr line format.

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
fn no_state_map_is_left_behind_a_lock() {
    // ARCH-02 B-5 moved the audio clips — the last `RwLock`-guarded map
    // the callback `try_read` — into the render graph. The table is empty
    // until B-6 deletes it.
    assert_eq!(STATE_MAP_COUNT, 0);
    assert!(StateMap::ALL.is_empty());
    assert_eq!(SharedState::default().lock_misses.snapshot(), [0u64; 0]);
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
        lock_misses_window: [],
        lock_misses_lifetime: [],
    };
    slot.publish(&report);
    assert_eq!(slot.take_new(&mut seen), Some(report.clone()));
    assert_eq!(slot.take_new(&mut seen), None, "same report is not re-printed");

    let mut next = report.clone();
    next.lock_skips_window = 0;
    slot.publish(&next);
    assert_eq!(slot.take_new(&mut seen), Some(next));
    assert_eq!(slot.take_new(&mut seen), None);
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
fn a_playing_block_always_renders_and_never_misses() {
    // Every map the playing branch reads is in the render graph (ARCH-02
    // B-5 moved the last one, the clips): a load that cannot miss, so
    // nothing is counted and nothing is skipped.
    let mut h = playing_harness();
    for _ in 0..4 {
        assert!(h.render().iter().any(|&s| s != 0.0));
    }
    assert_eq!(h.shared().lock_misses.snapshot(), [0; STATE_MAP_COUNT]);
    assert_eq!(h.shared().render_skip_cycles.load(Ordering::Relaxed), 0);

    // The skipped-block path survives only behind the test hook (B-6
    // deletes it): silence, one skip counted, still no lock miss.
    assert!(h.render_lock_contended().iter().all(|&s| s == 0.0));
    assert_eq!(h.shared().render_skip_cycles.load(Ordering::Relaxed), 1);
    assert_eq!(h.shared().lock_misses.snapshot(), [0; STATE_MAP_COUNT]);
    assert!(h.render().iter().any(|&s| s != 0.0));
}

#[test]
fn stopped_branch_takes_no_state_lock() {
    // The stopped branch reads tracks and plugin instances from the
    // render graph (ARCH-02 A2-6/A2-7) and never touches the clip list, so
    // it can neither miss nor skip.
    let mut h = playing_harness();
    h.shared().playing.store(false, Ordering::Relaxed);
    h.render();
    assert_eq!(h.shared().lock_misses.snapshot(), [0; STATE_MAP_COUNT]);
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
        lock_misses_window: [],
        lock_misses_lifetime: [],
    });
    // No locked map is left (ARCH-02 B-5), so no lock-miss segment.
    assert_eq!(
        line,
        "audio: dsp load avg 3.2% peak 41.0% | over-budget cycles 2 (lifetime 15) | monitor shortfalls 0 (lifetime 3) | render lock-skips 1 (lifetime 6)"
    );
}
