//! Tests for the real-time streaming pitch tracker.

use resonance_dsp::{PitchEstimate, PitchTracker, SimpleRng};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::f64::consts::TAU;
use std::time::Instant;

const SR: f32 = 48_000.0;
const BLOCK: usize = 512;

// --- Per-thread allocation counter for the no-allocation guard. -------

thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

/// System allocator that counts alloc/realloc calls per thread, so the
/// no-allocation guard is not polluted by concurrently running tests.
struct CountingAlloc;

// SAFETY: delegates entirely to `System`; the const-initialised
// thread-local counter never allocates itself (`try_with` tolerates TLS
// teardown).
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

fn thread_allocs() -> usize {
    ALLOC_COUNT.with(|c| c.get())
}

// --- Signal helpers. --------------------------------------------------

fn sine(freq: f64, len: usize, phase0: f64) -> (Vec<f32>, f64) {
    let mut phase = phase0;
    let step = TAU * freq / SR as f64;
    let out = (0..len)
        .map(|_| {
            let s = phase.sin() as f32 * 0.5;
            phase += step;
            s
        })
        .collect();
    (out, phase % TAU)
}

fn sawtooth(freq: f64, len: usize, phase0: f64) -> (Vec<f32>, f64) {
    // Naive sawtooth in [-0.5, 0.5]; the tracker's 1 kHz front-end
    // lowpass removes most of the aliased partials.
    let mut phase = phase0; // cycles in [0, 1)
    let step = freq / SR as f64;
    let out = (0..len)
        .map(|_| {
            let s = (phase - 0.5) as f32;
            phase = (phase + step) % 1.0;
            s
        })
        .collect();
    (out, phase)
}

/// Feed `signal` in BLOCK-sized chunks; returns the final estimate.
fn feed_all(tracker: &mut PitchTracker, signal: &[f32]) -> PitchEstimate {
    let mut est = tracker.latest();
    for chunk in signal.chunks(BLOCK) {
        est = tracker.feed(chunk);
    }
    est
}

fn cents_off(est_hz: f32, true_hz: f64) -> f64 {
    1200.0 * (f64::from(est_hz) / true_hz).log2()
}

// --- Tests. -----------------------------------------------------------

#[test]
fn tracks_stepped_sine_within_one_percent() {
    let mut tracker = PitchTracker::new(SR);
    let mut phase = 0.0;
    for &freq in &[80.0_f64, 120.0, 180.0, 250.0, 350.0, 450.0, 600.0] {
        let (signal, next_phase) = sine(freq, (SR * 0.7) as usize, phase);
        phase = next_phase;
        let est = feed_all(&mut tracker, &signal);
        assert!(est.voiced, "sine {freq} Hz must be voiced once settled");
        let rel = (f64::from(est.f0_hz) - freq).abs() / freq;
        assert!(
            rel < 0.01,
            "sine {freq} Hz: estimated {} Hz ({:.1} cents off)",
            est.f0_hz,
            cents_off(est.f0_hz, freq)
        );
        let expected_period = SR as f64 / freq;
        let period_rel = (f64::from(est.period_samples) - expected_period).abs() / expected_period;
        assert!(
            period_rel < 0.01,
            "sine {freq} Hz: period {} vs {expected_period}",
            est.period_samples
        );
    }
}

#[test]
fn tracks_stepped_sawtooth_within_one_percent() {
    let mut tracker = PitchTracker::new(SR);
    let mut phase = 0.0;
    for &freq in &[80.0_f64, 120.0, 180.0, 250.0, 350.0, 450.0, 600.0] {
        let (signal, next_phase) = sawtooth(freq, (SR * 0.7) as usize, phase);
        phase = next_phase;
        let est = feed_all(&mut tracker, &signal);
        assert!(est.voiced, "saw {freq} Hz must be voiced once settled");
        let rel = (f64::from(est.f0_hz) - freq).abs() / freq;
        assert!(
            rel < 0.01,
            "saw {freq} Hz: estimated {} Hz ({:.1} cents off)",
            est.f0_hz,
            cents_off(est.f0_hz, freq)
        );
    }
}

#[test]
fn strong_second_harmonic_does_not_cause_octave_error() {
    // Fundamental 150 Hz with a *louder* second harmonic: an
    // octave-error-prone spectrum. The NSDF key-maximum rule must still
    // report 150 Hz, not 300 Hz.
    let f0 = 150.0_f64;
    let n = (SR * 0.8) as usize;
    let signal: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f64 / SR as f64;
            (0.4 * (TAU * f0 * t).sin() + 0.56 * (TAU * 2.0 * f0 * t + 0.3).sin()) as f32
        })
        .collect();
    let mut tracker = PitchTracker::new(SR);
    let est = feed_all(&mut tracker, &signal);
    assert!(est.voiced);
    let rel = (f64::from(est.f0_hz) - f0).abs() / f0;
    assert!(
        rel < 0.01,
        "estimated {} Hz for a 150 Hz fundamental",
        est.f0_hz
    );
}

#[test]
fn white_noise_is_unvoiced() {
    let mut rng = SimpleRng::new(0xDEC1_0DE5);
    let signal: Vec<f32> = (0..(SR as usize))
        .map(|_| (rng.next_u32() as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32 * 0.5)
        .collect();
    let mut tracker = PitchTracker::new(SR);
    let mut voiced_blocks = 0;
    let mut total_blocks = 0;
    let mut markers = 0;
    for chunk in signal.chunks(BLOCK) {
        let est = tracker.feed(chunk);
        voiced_blocks += usize::from(est.voiced);
        markers += tracker.markers().len();
        total_blocks += 1;
    }
    assert!(
        voiced_blocks * 20 < total_blocks,
        "white noise voiced in {voiced_blocks}/{total_blocks} blocks"
    );
    assert!(!tracker.latest().voiced, "noise must end unvoiced");
    assert!(
        markers < total_blocks / 10,
        "noise emitted {markers} period markers"
    );
}

#[test]
fn silence_is_unvoiced_with_no_markers() {
    let mut tracker = PitchTracker::new(SR);
    let silence = vec![0.0_f32; SR as usize];
    let est = feed_all(&mut tracker, &silence);
    assert!(!est.voiced);
    assert!(est.f0_hz == 0.0 && est.period_samples == 0.0);
    assert!(tracker.markers().is_empty());
}

#[test]
fn marker_spacing_matches_the_true_period() {
    let freq = 220.0_f64;
    let true_period = SR as f64 / freq; // ≈ 218.18 samples
    let (signal, _) = sine(freq, (SR * 2.0) as usize, 0.0);
    let mut tracker = PitchTracker::new(SR);
    let mut markers: Vec<f64> = Vec::new();
    for chunk in signal.chunks(BLOCK) {
        tracker.feed(chunk);
        markers.extend_from_slice(tracker.markers());
    }
    // Ignore the settling half-second, then check every gap.
    let settled: Vec<f64> = markers
        .iter()
        .copied()
        .filter(|&m| m > 0.5 * f64::from(SR))
        .collect();
    assert!(
        settled.len() > 200,
        "expected a dense marker train, got {}",
        settled.len()
    );
    for pair in settled.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            (gap - true_period).abs() < 2.0,
            "marker gap {gap} vs true period {true_period}"
        );
    }
    // The train covers the settled region at roughly one marker per period.
    let span = settled.last().unwrap() - settled.first().unwrap();
    let expected = span / true_period;
    let got = (settled.len() - 1) as f64;
    assert!(
        (got - expected).abs() < 2.0,
        "marker count {got} vs expected {expected}"
    );
}

#[test]
fn feed_does_not_allocate() {
    let mut tracker = PitchTracker::new(SR);
    // Warm up into a voiced state so the guarded region covers analysis,
    // marker emission and the voiced steady state.
    let (warmup, phase) = sine(220.0, (SR * 0.5) as usize, 0.0);
    feed_all(&mut tracker, &warmup);
    assert!(tracker.latest().voiced);

    let (signal, _) = sine(220.0, (SR * 0.5) as usize, phase);
    let before = thread_allocs();
    for chunk in signal.chunks(BLOCK) {
        tracker.feed(chunk);
        std::hint::black_box(tracker.markers());
    }
    let after = thread_allocs();
    assert!(
        after == before,
        "feed() allocated {} times",
        after - before
    );
}

#[test]
fn processes_ten_seconds_well_under_real_time() {
    let (signal, _) = sine(220.0, (SR * 10.0) as usize, 0.0);
    let mut tracker = PitchTracker::new(SR);
    let start = Instant::now();
    let est = feed_all(&mut tracker, &signal);
    let elapsed = start.elapsed();
    std::hint::black_box(est);
    assert!(
        elapsed.as_secs_f64() < 5.0,
        "10 s of audio took {elapsed:?} — not comfortably real-time"
    );
}

#[test]
fn reset_returns_to_a_clean_unvoiced_state() {
    let mut tracker = PitchTracker::new(SR);
    let (signal, _) = sine(220.0, (SR * 0.5) as usize, 0.0);
    assert!(feed_all(&mut tracker, &signal).voiced);
    tracker.reset();
    let est = tracker.latest();
    assert!(!est.voiced && est.f0_hz == 0.0);
    assert!(tracker.markers().is_empty());
    // And it tracks again after the reset.
    let (signal, _) = sine(330.0, (SR * 0.7) as usize, 0.0);
    let est = feed_all(&mut tracker, &signal);
    assert!(est.voiced);
    assert!((f64::from(est.f0_hz) - 330.0).abs() / 330.0 < 0.01);
}
