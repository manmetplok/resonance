//! Shimmer (pitch inside the feedback loop) and per-grain pitch
//! quantization through the full plugin stack (ba todo #1078, doc #252
//! §3): cumulative octave climb with FB Pitch on, constant-pitch
//! repeats with it off, the freeze interaction (the climb pauses while
//! the write head is stopped), exact semitone/scale rate lattices under
//! quantization and the no-allocation guarantee with everything active.

use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

const SR: f32 = 48_000.0;

// --- Per-thread allocation counter for the no-allocation guard --------
// (pattern from tests/plugin.rs / tests/feedback.rs / tests/stereo.rs).

thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

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

// --- Helpers ----------------------------------------------------------

fn run_blocks(
    plugin: &mut ResonanceGranularDelay,
    left: &mut [f32],
    right: &mut [f32],
    block: usize,
) {
    let frames = left.len();
    let mut pos = 0;
    while pos < frames {
        let n = (frames - pos).min(block);
        let mut outs = [OutputBuffer {
            left: &mut left[pos..pos + n],
            right: &mut right[pos..pos + n],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        pos += n;
    }
}

/// Deterministic wet-only plugin (Sync scheduler, zero jitters, damping
/// wide open) for shimmer renders.
fn shimmer_plugin(time_ms: f32, feedback: f32) -> ResonanceGranularDelay {
    let plugin = ResonanceGranularDelay::new();
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_ms.set_value(time_ms);
    plugin.params.grain_size_ms.set_value(80.0);
    plugin.params.density_hz.set_value(30.0);
    plugin.params.scheduler.set_value(0); // Sync (deterministic)
    plugin.params.spray_ms.set_value(0.0);
    plugin.params.size_jitter.set_value(0.0);
    plugin.params.level_jitter.set_value(0.0);
    plugin.params.pan_spread.set_value(0.0);
    plugin.params.reverse_prob.set_value(0.0);
    plugin.params.spread_cents.set_value(0.0);
    plugin.params.mix.set_value(1.0); // wet only
    plugin.params.feedback.set_value(feedback);
    plugin.params.fb_route.set_value(0); // Wet -> Buffer
    plugin.params.filter_hz.set_value(20_000.0);
    plugin
}

/// Goertzel single-bin power, normalized by window length.
fn goertzel(x: &[f32], freq: f32, sr: f32) -> f64 {
    let w = std::f64::consts::TAU * freq as f64 / sr as f64;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &v in x {
        let s0 = v as f64 + c * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - c * s1 * s2) / x.len().max(1) as f64
}

fn zero_crossings(x: &[f32]) -> usize {
    x.windows(2)
        .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
        .count()
}

/// Render a 150 ms 220 Hz burst through a shimmer plugin (+12 st,
/// 400 ms delay, 90 % feedback) and return the left channel.
fn render_burst(fb_pitch: bool) -> Vec<f32> {
    let mut plugin = shimmer_plugin(400.0, 0.9);
    plugin.params.pitch.set_value(12.0);
    plugin
        .params
        .fb_pitch
        .set_plain(if fb_pitch { 1.0 } else { 0.0 });
    plugin.initialize(SR, 512);

    let frames = (2.5 * SR) as usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    for i in 0..(0.15 * SR) as usize {
        let s = (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin() * 0.8;
        left[i] = s;
        right[i] = s;
    }
    run_blocks(&mut plugin, &mut left, &mut right, 512);
    left
}

/// Repeat-`k` analysis window (loop period = delay + one block of
/// feedback-bus latency, as in tests/feedback.rs).
fn repeat_window(x: &[f32], k: usize) -> &[f32] {
    let period = (0.400 * SR) as usize + 512;
    &x[k * period..k * period + (0.12 * SR) as usize]
}

// --- Tests ------------------------------------------------------------

/// DoD: FB Pitch on with Pitch +12 on a 220 Hz burst — successive
/// repeats show spectral peaks an octave apart: repeat 1 at 440 Hz,
/// repeat 2 at 880 Hz, repeat 3 at 1760 Hz (f, 2f, 4f).
#[test]
fn shimmer_on_climbs_an_octave_per_repeat() {
    let out = render_burst(true);
    for (k, f) in [(1usize, 440.0f32), (2, 880.0), (3, 1760.0)] {
        let win = repeat_window(&out, k);
        let here = goertzel(win, f, SR);
        let below = goertzel(win, f * 0.5, SR);
        let above = goertzel(win, f * 2.0, SR);
        assert!(
            here > 1e-9,
            "repeat {k} has no energy at {f} Hz: {here}"
        );
        assert!(
            here > 3.0 * below && here > 3.0 * above,
            "repeat {k} not dominated by {f} Hz: at {f}: {here}, \
             octave below: {below}, octave above: {above}"
        );
    }
}

/// DoD: FB Pitch off — the transpose is heard exactly once and
/// recirculations keep a constant pitch: repeats 1..3 all peak at
/// 440 Hz, never climbing to 880/1760 Hz.
#[test]
fn fb_pitch_off_keeps_repeats_at_constant_pitch() {
    let out = render_burst(false);
    for k in 1..=3usize {
        let win = repeat_window(&out, k);
        let at_440 = goertzel(win, 440.0, SR);
        let at_880 = goertzel(win, 880.0, SR);
        let at_1760 = goertzel(win, 1760.0, SR);
        assert!(
            at_440 > 1e-9,
            "repeat {k} has no energy at 440 Hz: {at_440}"
        );
        assert!(
            at_440 > 3.0 * at_880 && at_440 > 3.0 * at_1760,
            "repeat {k} climbed instead of holding 440 Hz: \
             440: {at_440}, 880: {at_880}, 1760: {at_1760}"
        );
    }
}

/// With FB Pitch off, the recirculating signal comes from the
/// dedicated un-transposed tap engines: their grains all run at
/// exactly rate 1 while the audible grains run at rate 2 (+12 st).
#[test]
fn fb_tap_grains_are_untransposed_while_audible_grains_shift() {
    let mut plugin = shimmer_plugin(300.0, 0.9);
    plugin.params.pitch.set_value(12.0);
    plugin.params.fb_pitch.set_plain(0.0);
    plugin.initialize(SR, 512);

    let frames = SR as usize;
    let mut left = vec![0.4f32; frames];
    let mut right = vec![0.4f32; frames];
    run_blocks(&mut plugin, &mut left, &mut right, 512);

    let audible: Vec<f64> = plugin.active_rates().collect();
    let tap: Vec<f64> = plugin.active_rates_fb().collect();
    assert!(
        !audible.is_empty() && !tap.is_empty(),
        "expected live grains in both engines: audible {}, tap {}",
        audible.len(),
        tap.len()
    );
    for r in &audible {
        assert!(
            (r - 2.0).abs() < 1e-9,
            "audible grain not at rate 2 (+12 st): {r}"
        );
    }
    for r in &tap {
        assert!(
            (r - 1.0).abs() < 1e-12,
            "feedback-tap grain transposed: rate {r}"
        );
    }
}

/// Freeze interaction (ba todos #1075/#1078): freezing stops the write
/// head, so nothing new recirculates and the shimmer climb pauses —
/// the frozen wet output is pitch-stationary (equal zero-crossing
/// rates in two windows a second apart) instead of climbing further.
#[test]
fn freeze_pauses_the_shimmer_climb() {
    let mut plugin = shimmer_plugin(400.0, 0.9);
    plugin.params.pitch.set_value(12.0);
    plugin.params.fb_pitch.set_plain(1.0);
    plugin.initialize(SR, 512);

    let frames = (3.0 * SR) as usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    for i in 0..(0.15 * SR) as usize {
        let s = (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin() * 0.8;
        left[i] = s;
        right[i] = s;
    }

    // Stream the first second (burst + first repeats), then freeze.
    let split = SR as usize;
    run_blocks(&mut plugin, &mut left[..split], &mut right[..split], 512);
    plugin.params.freeze.set_plain(1.0);
    run_blocks(&mut plugin, &mut left[split..], &mut right[split..], 512);

    let win_a = &left[(1.3 * SR) as usize..(1.7 * SR) as usize];
    let win_b = &left[(2.3 * SR) as usize..(2.7 * SR) as usize];
    let (rms_a, rms_b) = (
        (win_a.iter().map(|v| v * v).sum::<f32>() / win_a.len() as f32).sqrt(),
        (win_b.iter().map(|v| v * v).sum::<f32>() / win_b.len() as f32).sqrt(),
    );
    assert!(
        rms_a > 1e-4 && rms_b > 1e-4,
        "freeze did not sustain the shimmer tail: rms {rms_a} / {rms_b}"
    );
    let (zc_a, zc_b) = (zero_crossings(win_a), zero_crossings(win_b));
    let (lo, hi) = (zc_a.min(zc_b) as f32, zc_a.max(zc_b) as f32);
    assert!(
        hi <= lo * 1.15,
        "spectral content kept moving while frozen: {zc_a} vs {zc_b} zero crossings"
    );
}

/// Rates collected across a run with the given pitch/quantize/scale
/// settings, plus the rendered left channel for spectral checks.
fn quantized_run(
    pitch: f32,
    spread_cents: f32,
    quantize: i32,
    root: i32,
    scale: i32,
) -> (Vec<f64>, Vec<f32>) {
    let plugin = ResonanceGranularDelay::new();
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_ms.set_value(200.0);
    plugin.params.grain_size_ms.set_value(120.0);
    plugin.params.density_hz.set_value(30.0);
    plugin.params.scheduler.set_value(0); // Sync
    plugin.params.spray_ms.set_value(0.0);
    plugin.params.size_jitter.set_value(0.0);
    plugin.params.level_jitter.set_value(0.0);
    plugin.params.pan_spread.set_value(0.0);
    plugin.params.reverse_prob.set_value(0.0);
    plugin.params.feedback.set_value(0.0);
    plugin.params.mix.set_value(1.0);
    plugin.params.pitch.set_value(pitch);
    plugin.params.spread_cents.set_value(spread_cents);
    plugin.params.pitch_quantize.set_value(quantize);
    plugin.params.root.set_value(root);
    plugin.params.scale.set_value(scale);
    let mut plugin = plugin;
    plugin.initialize(SR, 512);

    let frames = (1.5 * SR) as usize;
    let mut left: Vec<f32> = (0..frames)
        .map(|i| (std::f32::consts::TAU * 880.0 * i as f32 / SR).sin() * 0.6)
        .collect();
    let mut right = left.clone();

    let mut rates = Vec::new();
    let mut pos = 0;
    while pos < frames {
        let n = (frames - pos).min(512);
        let mut outs = [OutputBuffer {
            left: &mut left[pos..pos + n],
            right: &mut right[pos..pos + n],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        rates.extend(plugin.active_rates());
        pos += n;
    }
    (rates, left)
}

/// Energy-weighted spectral centroid over `lo..hi` Hz, evaluated on a
/// Goertzel grid. Granulating a steady sine with a *periodic* (Sync)
/// scheduler yields a comb spectrum whose line positions depend only on
/// the source and the grain rate's periodicity — the transpose moves
/// the spectral *envelope*, which the centroid tracks.
fn band_centroid(x: &[f32], lo: f32, hi: f32, step: f32, sr: f32) -> f64 {
    let (mut num, mut den) = (0.0f64, 0.0f64);
    let mut f = lo;
    while f <= hi {
        let p = goertzel(x, f, sr);
        num += f as f64 * p;
        den += p;
        f += step;
    }
    num / den.max(1e-30)
}

/// DoD: semitone quantization lands every grain on the exact semitone
/// lattice — pitch 2.4 st snaps to +2 st: every grain rate equals
/// 2^(2/12) to 1e-6 and the wet spectral envelope sits at
/// 880·2^(2/12) = 987.8 Hz, measurably below the free-running
/// envelope at 880·2^(2.4/12) = 1010.9 Hz.
#[test]
fn semitone_quantize_lands_on_exact_semitone_frequencies() {
    let steady = |x: &[f32]| x[(0.4 * SR) as usize..].to_vec();

    let (rates_q, out_q) = quantized_run(2.4, 0.0, 1, 0, 1);
    assert!(rates_q.len() > 50, "too few grains observed: {}", rates_q.len());
    let lattice_rate = 2f64.powf(2.0 / 12.0);
    for r in &rates_q {
        assert!(
            (r - lattice_rate).abs() < 1e-6,
            "quantized grain off the +2 st lattice: rate {r}"
        );
    }

    let (rates_f, out_f) = quantized_run(2.4, 0.0, 0, 0, 1);
    let free_rate = 2f64.powf(2.4 / 12.0);
    for r in &rates_f {
        assert!(
            (r - free_rate).abs() < 1e-6,
            "quantize off altered the grain rate: {r}"
        );
    }

    // Spectral envelope: the quantized render centres ~23 Hz below the
    // free-running one (987.8 vs 1010.9 Hz target regions).
    let c_q = band_centroid(&steady(&out_q), 900.0, 1100.0, 4.0, SR);
    let c_f = band_centroid(&steady(&out_f), 900.0, 1100.0, 4.0, SR);
    assert!(
        c_f - c_q > 10.0,
        "quantization did not move the spectral envelope down to the \
         semitone: quantized centroid {c_q} Hz, free centroid {c_f} Hz"
    );
}

/// DoD: scale quantization uses the resonance-music-theory degree
/// lattice — pitch 3.4 st snaps to a major third (+4 st) in C major
/// but a minor third (+3 st) in A minor, exactly (rates to 1e-6), and
/// the spectral envelopes land a semitone apart (1108.7 vs 1046.5 Hz).
#[test]
fn scale_quantize_follows_the_configured_scale() {
    let steady = |x: &[f32]| x[(0.4 * SR) as usize..].to_vec();
    let (rates_major, out_major) = quantized_run(3.4, 0.0, 2, 0, 1); // C major
    let (rates_minor, out_minor) = quantized_run(3.4, 0.0, 2, 9, 2); // A minor
    assert!(!rates_major.is_empty() && !rates_minor.is_empty());
    let major_third = 2f64.powf(4.0 / 12.0);
    let minor_third = 2f64.powf(3.0 / 12.0);
    for r in &rates_major {
        assert!(
            (r - major_third).abs() < 1e-6,
            "C-major grain not on the +4 st degree: rate {r}"
        );
    }
    for r in &rates_minor {
        assert!(
            (r - minor_third).abs() < 1e-6,
            "A-minor grain not on the +3 st degree: rate {r}"
        );
    }
    let c_major = band_centroid(&steady(&out_major), 980.0, 1180.0, 4.0, SR);
    let c_minor = band_centroid(&steady(&out_minor), 980.0, 1180.0, 4.0, SR);
    assert!(
        c_major - c_minor > 30.0,
        "major/minor thirds did not separate spectrally: \
         C-major centroid {c_major} Hz, A-minor centroid {c_minor} Hz"
    );
}

/// Quantizing the random Spread turns the detune cloud into discrete
/// lattice steps: with ±100 cents of spread every grain sits on
/// exactly -1, 0 or +1 st (and more than one step actually occurs),
/// while quantize off leaves fractional detunes in between.
#[test]
fn quantized_spread_collapses_to_lattice_steps() {
    let lattice = [2f64.powf(-1.0 / 12.0), 1.0, 2f64.powf(1.0 / 12.0)];

    let (rates_q, _) = quantized_run(0.0, 100.0, 1, 0, 1);
    assert!(rates_q.len() > 50, "too few grains observed: {}", rates_q.len());
    for r in &rates_q {
        assert!(
            lattice.iter().any(|l| (r - l).abs() < 1e-6),
            "quantized spread produced an off-lattice rate: {r}"
        );
    }
    let distinct = lattice
        .iter()
        .filter(|l| rates_q.iter().any(|r| (r - **l).abs() < 1e-6))
        .count();
    assert!(
        distinct >= 2,
        "quantized spread never varied across the lattice: {distinct} step(s)"
    );

    let (rates_f, _) = quantized_run(0.0, 100.0, 0, 0, 1);
    let off_lattice = rates_f.iter().any(|r| {
        let semis = 12.0 * r.log2();
        (semis - semis.round()).abs() > 0.1
    });
    assert!(
        off_lattice,
        "quantize off produced no fractional detunes to distinguish from the lattice"
    );
}

/// The audio path stays allocation-free with shimmer, the un-transposed
/// feedback tap and scale quantization all active, including toggling
/// FB Pitch mid-run (tap engage/drain) and quantize off again.
#[test]
fn shimmer_and_quantize_do_not_allocate() {
    let mut plugin = shimmer_plugin(200.0, 0.9);
    plugin.params.pitch.set_value(7.3);
    plugin.params.spread_cents.set_value(60.0);
    plugin.params.pitch_quantize.set_value(2); // Scale
    plugin.params.root.set_value(2);
    plugin.params.scale.set_value(3);
    plugin.params.fb_pitch.set_plain(0.0); // un-transposed tap active
    plugin.params.mix.set_value(0.5);
    plugin.initialize(SR, 512);

    let block = 512usize;
    let mut left = vec![0.3f32; block * 40];
    let mut right = vec![0.3f32; block * 40];

    // Warm-up block.
    {
        let (l, r) = (&mut left[..block], &mut right[..block]);
        let mut outs = [OutputBuffer { left: l, right: r }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, block, &mut ev, None);
    }

    let before = thread_allocs();
    run_blocks(&mut plugin, &mut left, &mut right, block);
    plugin.params.fb_pitch.set_plain(1.0); // tap -> drain, shimmer on
    run_blocks(&mut plugin, &mut left, &mut right, block);
    plugin.params.pitch_quantize.set_value(0); // back to free pitch
    plugin.params.fb_pitch.set_plain(0.0); // tap re-engages
    run_blocks(&mut plugin, &mut left, &mut right, block);
    let after = thread_allocs();
    assert_eq!(
        after - before,
        0,
        "shimmer/quantize path allocated {} times on the audio path",
        after - before
    );
}
