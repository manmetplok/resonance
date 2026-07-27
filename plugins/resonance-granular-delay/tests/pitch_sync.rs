//! Pitch-synchronous Voice/Mono scheduler (ba todo #1082, doc #252
//! §4): PSOLA-style grain placement on monophonic material. Onsets lock
//! to the tracked period (IOT jitter collapses vs the async
//! scheduler's designed ±50% spread), the output envelope flattens vs
//! async granulation at equivalent settings, transposition shifts f0 by
//! onset-spacing while the spectral envelope (formant proxy: band
//! centroid) stays put — unlike the async cloud's per-grain resampling
//! — noise falls back to the async scheduler without a dropout, freeze
//! keeps the drone spawning from the last known period, and the audio
//! path stays allocation-free with the voice path active.

use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

const SR: f32 = 48_000.0;

// --- Per-thread allocation counter for the no-allocation guard --------

thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

struct CountingAlloc;

// SAFETY: delegates entirely to `System`; the const-initialised
// thread-local counter never allocates itself.
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

/// Deterministic wet-only plugin with the given scheduler (0 = Sync,
/// 1 = Async, 2 = Pitch-Sync).
fn voice_plugin(scheduler: i32, time_ms: f32, pitch: f32) -> ResonanceGranularDelay {
    let plugin = ResonanceGranularDelay::new();
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_ms.set_value(time_ms);
    plugin.params.grain_size_ms.set_value(90.0);
    plugin.params.density_hz.set_value(25.0);
    plugin.params.scheduler.set_value(scheduler);
    plugin.params.pitch.set_value(pitch);
    plugin.params.spray_ms.set_value(0.0);
    plugin.params.size_jitter.set_value(0.0);
    plugin.params.level_jitter.set_value(0.0);
    plugin.params.pan_spread.set_value(0.0);
    plugin.params.reverse_prob.set_value(0.0);
    plugin.params.spread_cents.set_value(0.0);
    plugin.params.feedback.set_value(0.0);
    plugin.params.mix.set_value(1.0); // wet only
    plugin
}

/// Naive sawtooth with optional vibrato (depth in semitones).
fn saw(frames: usize, f0: f32, amp: f32, vib_st: f32, vib_hz: f32) -> Vec<f32> {
    let mut phase = 0.0f64;
    (0..frames)
        .map(|i| {
            let t = i as f64 / SR as f64;
            let vib = f64::from(vib_st) * (std::f64::consts::TAU * f64::from(vib_hz) * t).sin();
            let f = f64::from(f0) * (vib / 12.0).exp2();
            phase = (phase + f / SR as f64).fract();
            ((2.0 * phase - 1.0) as f32) * amp
        })
        .collect()
}

/// Deterministic uniform noise in ±amp (xorshift).
fn noise(frames: usize, amp: f32, mut seed: u64) -> Vec<f32> {
    (0..frames)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            ((seed >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0) * amp
        })
        .collect()
}

/// Two-pole resonator (a synthetic formant) followed by peak
/// normalization to `peak` — turns a saw into a sung-vowel-like source.
fn resonate(x: &[f32], freq: f32, r: f32, peak: f32) -> Vec<f32> {
    let theta = std::f32::consts::TAU * freq / SR;
    let (a1, a2) = (2.0 * r * theta.cos(), -r * r);
    let (mut y1, mut y2) = (0.0f32, 0.0f32);
    let mut out: Vec<f32> = x
        .iter()
        .map(|&v| {
            let y = v + a1 * y1 + a2 * y2;
            y2 = y1;
            y1 = y;
            y
        })
        .collect();
    let max = out.iter().fold(0.0f32, |m, &v| m.max(v.abs())).max(1e-9);
    for v in out.iter_mut() {
        *v *= peak / max;
    }
    out
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// RMS envelope over consecutive `win`-sample windows.
fn envelope(x: &[f32], win: usize) -> Vec<f32> {
    x.chunks_exact(win).map(rms).collect()
}

/// Envelope modulation depth: std / mean of the window-RMS envelope.
fn modulation_depth(env: &[f32]) -> f64 {
    let mean = env.iter().map(|&v| f64::from(v)).sum::<f64>() / env.len() as f64;
    let var = env
        .iter()
        .map(|&v| (f64::from(v) - mean).powi(2))
        .sum::<f64>()
        / env.len() as f64;
    var.sqrt() / mean.max(1e-12)
}

/// Goertzel magnitude of `x` at `freq`.
fn goertzel(x: &[f32], freq: f32) -> f64 {
    let w = std::f64::consts::TAU * f64::from(freq) / f64::from(SR);
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &v in x {
        let s0 = f64::from(v) + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt() / x.len() as f64
}

/// Magnitude-weighted spectral centroid over `[lo, hi]` Hz in `step`
/// Hz bins — the formant proxy.
fn spectral_centroid(x: &[f32], lo: f32, hi: f32, step: f32) -> f64 {
    let (mut num, mut den) = (0.0f64, 0.0f64);
    let mut f = lo;
    while f <= hi {
        let m = goertzel(x, f);
        num += f64::from(f) * m;
        den += m;
        f += step;
    }
    num / den.max(1e-12)
}

/// Dominant period in samples via the autocorrelation peak over
/// `[min_lag, max_lag]`.
fn dominant_period(x: &[f32], min_lag: usize, max_lag: usize) -> usize {
    let n = x.len() - max_lag;
    let mut best = (min_lag, f64::MIN);
    for lag in min_lag..=max_lag {
        let r: f64 = (0..n).map(|i| f64::from(x[i]) * f64::from(x[i + lag])).sum();
        if r > best.1 {
            best = (lag, r);
        }
    }
    best.0
}

// --- Tests ------------------------------------------------------------

/// On a sung-vowel-like synthetic (sawtooth + mild vibrato) the voice
/// path engages, tracks the fundamental, and locks grain onsets to the
/// period lattice: mean inter-onset time matches the tracked period
/// within 3% and the IOT spread collapses to < 5% CV — an order below
/// the async scheduler's designed ±50% (≈29% CV) onset jitter.
#[test]
fn voiced_input_engages_and_onsets_lock_to_the_tracked_period() {
    let frames = (4.0 * SR) as usize;
    let input = saw(frames, 160.0, 0.6, 0.3, 5.0);
    let mut plugin = voice_plugin(2, 250.0, 0.0);
    plugin.initialize(SR, 4096);
    let mut left = input.clone();
    let mut right = input;
    run_blocks(&mut plugin, &mut left, &mut right, 512);

    assert!(plugin.pitch_sync_engaged(), "voice path never engaged");
    let period = plugin.tracked_period_samples();
    let nominal = SR / 160.0; // 300 samples
    assert!(
        (period - nominal).abs() < 0.05 * nominal,
        "tracked period off: {period} vs nominal {nominal}"
    );
    assert!(plugin.psola_onsets() > 100, "too few PSOLA onsets");

    let onsets = plugin.psola_recent_onsets();
    assert!(onsets.len() >= 32, "onset log too short: {}", onsets.len());
    let iot: Vec<f64> = onsets.windows(2).map(|w| w[1] - w[0]).collect();
    let mean = iot.iter().sum::<f64>() / iot.len() as f64;
    assert!(
        (mean - f64::from(period)).abs() < 0.03 * f64::from(period),
        "mean IOT {mean} does not match tracked period {period}"
    );
    let var = iot.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / iot.len() as f64;
    let cv = var.sqrt() / mean;
    assert!(
        cv < 0.05,
        "IOT jitter did not collapse: CV {cv} (async designs ~0.29)"
    );
}

/// Pitch-Sync at transpose 0 has clearly lower output envelope
/// modulation than the async scheduler at equivalent settings — the
/// period-locked Hann train overlap-adds to a near-constant envelope,
/// where fixed-rate granulation of pitched material beats against f0.
#[test]
fn pitch_sync_flattens_the_output_envelope_vs_async() {
    let frames = (4.0 * SR) as usize;
    let input = saw(frames, 150.0, 0.6, 0.0, 0.0);
    let render = |scheduler: i32| -> Vec<f32> {
        let mut plugin = voice_plugin(scheduler, 200.0, 0.0);
        plugin.initialize(SR, 4096);
        let mut left = input.clone();
        let mut right = input.clone();
        run_blocks(&mut plugin, &mut left, &mut right, 512);
        left
    };
    let ps = render(2);
    let async_out = render(1);

    let steady = (2.0 * SR) as usize..(3.8 * SR) as usize;
    let win = (0.02 * SR) as usize;
    assert!(rms(&ps[steady.clone()]) > 0.05, "pitch-sync wet too quiet");
    assert!(rms(&async_out[steady.clone()]) > 0.05, "async wet too quiet");
    let depth_ps = modulation_depth(&envelope(&ps[steady.clone()], win));
    let depth_async = modulation_depth(&envelope(&async_out[steady], win));
    assert!(
        depth_ps < 0.5 * depth_async,
        "envelope modulation did not collapse: pitch-sync {depth_ps} vs async {depth_async}"
    );
}

/// At +7 st the voice path shifts the output fundamental by ~7 st
/// (onset-spacing density, not resampling) while the spectral envelope
/// stays put: the band centroid of a vowel-like source (saw through an
/// 800 Hz formant resonator) moves < 20% under Pitch-Sync but ≥ 25%
/// under the async scheduler's per-grain resampling.
#[test]
fn transpose_shifts_f0_while_preserving_the_formant() {
    let frames = (4.0 * SR) as usize;
    let input = resonate(&saw(frames, 150.0, 0.6, 0.0, 0.0), 800.0, 0.97, 0.5);
    let render = |scheduler: i32| -> Vec<f32> {
        let mut plugin = voice_plugin(scheduler, 200.0, 7.0);
        plugin.initialize(SR, 4096);
        let mut left = input.clone();
        let mut right = input.clone();
        run_blocks(&mut plugin, &mut left, &mut right, 512);
        left
    };
    let ps = render(2);
    let async_out = render(1);

    let seg = |x: &[f32]| x[(2.5 * SR) as usize..(2.5 * SR) as usize + 8192].to_vec();
    let in_seg = seg(&input);
    let ps_seg = seg(&ps);
    let async_seg = seg(&async_out);

    // f0: input at 150 Hz (lag 320); +7 st ≈ ×1.4983 → lag ≈ 213.6.
    let in_lag = dominant_period(&in_seg, 100, 400);
    assert!(
        (300..=340).contains(&in_lag),
        "input period sanity failed: lag {in_lag}"
    );
    let ps_lag = dominant_period(&ps_seg, 100, 400);
    let ratio = in_lag as f64 / ps_lag as f64;
    assert!(
        (1.42..=1.58).contains(&ratio),
        "pitch-sync f0 shift wrong: lag {ps_lag} (ratio {ratio}, want ~1.498)"
    );

    // Formant proxy: band centroid 300–2500 Hz.
    let c_in = spectral_centroid(&in_seg, 300.0, 2500.0, 25.0);
    let c_ps = spectral_centroid(&ps_seg, 300.0, 2500.0, 25.0);
    let c_async = spectral_centroid(&async_seg, 300.0, 2500.0, 25.0);
    assert!(
        (c_ps - c_in).abs() < 0.20 * c_in,
        "pitch-sync moved the formant: centroid {c_ps} vs input {c_in}"
    );
    assert!(
        c_async > 1.25 * c_in,
        "async resampling contrast missing: centroid {c_async} vs input {c_in}"
    );
}

/// Feeding noise drops the tracker to unvoiced and hands over to the
/// async scheduler transparently: the async cloud resumes spawning, the
/// PSOLA pool drains until silent, and the wet output never gaps
/// through the transition.
#[test]
fn noise_input_falls_back_to_async_without_dropout() {
    let voiced = (2.0 * SR) as usize;
    let frames = (4.0 * SR) as usize;
    let mut input = saw(frames, 150.0, 0.6, 0.0, 0.0);
    let n = noise(frames - voiced, 0.4, 0x1082_5EED);
    input[voiced..].copy_from_slice(&n);

    let mut plugin = voice_plugin(2, 200.0, 0.0);
    plugin.initialize(SR, 4096);
    let mut left = input.clone();
    let mut right = input;

    // Voiced prefix: engaged, async cloud drained.
    run_blocks(&mut plugin, &mut left[..voiced], &mut right[..voiced], 512);
    assert!(plugin.pitch_sync_engaged(), "voice path never engaged");
    let async_spawns_at_handover = plugin.grains_spawned();

    let (l_rest, r_rest) = (&mut left[voiced..], &mut right[voiced..]);
    run_blocks(&mut plugin, l_rest, r_rest, 512);

    assert!(
        !plugin.pitch_sync_engaged(),
        "noise did not drop the voice path"
    );
    assert!(
        plugin.grains_spawned() > async_spawns_at_handover + 20,
        "async fallback never resumed spawning"
    );
    assert_eq!(
        plugin.psola_active_voices(),
        0,
        "stale PSOLA voices survived the fallback"
    );

    // No dropout through the handover: every 25 ms window of the first
    // post-transition second keeps a healthy fraction of the settled
    // fallback level.
    let win = (0.025 * SR) as usize;
    let steady = rms(&left[(3.0 * SR) as usize..(3.8 * SR) as usize]);
    assert!(steady > 0.02, "fallback wet too quiet: {steady}");
    let min_win = envelope(&left[voiced..voiced + SR as usize], win)
        .into_iter()
        .fold(f32::INFINITY, f32::min);
    assert!(
        min_win > 0.15 * steady,
        "wet output gapped during the fallback: min window rms {min_win} vs steady {steady}"
    );
}

/// Freeze stalls the write head and the tracker feed, but the voice
/// path keeps spawning from the last known period against the held
/// marker ring: the frozen drone sustains at the tracked pitch.
#[test]
fn freeze_holds_the_pitch_sync_drone_at_the_tracked_period() {
    let voiced = (2.0 * SR) as usize;
    let frozen = SR as usize;
    let mut plugin = voice_plugin(2, 200.0, 0.0);
    plugin.initialize(SR, 4096);

    let input = saw(voiced, 160.0, 0.6, 0.0, 0.0);
    let mut left = input.clone();
    let mut right = input;
    run_blocks(&mut plugin, &mut left, &mut right, 512);
    assert!(plugin.pitch_sync_engaged(), "voice path never engaged");
    let onsets_at_freeze = plugin.psola_onsets();

    plugin.params.freeze.set_plain(1.0);
    let mut fl = vec![0.0f32; frozen];
    let mut fr = vec![0.0f32; frozen];
    run_blocks(&mut plugin, &mut fl, &mut fr, 512);

    assert!(
        plugin.pitch_sync_engaged(),
        "freeze dropped the voice path"
    );
    assert!(
        plugin.psola_onsets() > onsets_at_freeze + 100,
        "frozen head stopped spawning from the last known period"
    );
    let tail = &fl[(0.2 * SR as f32) as usize..];
    assert!(rms(tail) > 0.05, "frozen drone died: rms {}", rms(tail));
    // The drone holds the tracked pitch (period 300 samples at 160 Hz).
    let lag = dominant_period(&tail[..8192 + 400], 200, 400);
    assert!(
        ((285..=315).contains(&lag)),
        "frozen drone period off: lag {lag} (want ~300)"
    );
}

/// Onset scheduling carries across process calls on absolute sample
/// counters, so slicing the same audio into 512- vs 128-sample blocks
/// leaves the engaged onset lattice in place — corresponding onsets
/// land within half a sample of each other (the tiny residual is the
/// block-latched period refinement, ~0.003 samples per analysis) — and
/// the rendered envelope is unchanged. (Bit-exactness across block
/// sizes is not claimed: the tracker estimate, like every block-latched
/// parameter, applies at block rate.)
#[test]
fn engaged_onset_lattice_is_block_size_invariant() {
    let prefix = (2.0 * SR) as usize;
    let cont = SR as usize;
    let input = saw(prefix + cont, 150.0, 0.6, 0.0, 0.0);

    let render_tail = |block: usize| -> (Vec<f32>, Vec<f64>) {
        let mut plugin = voice_plugin(2, 200.0, 5.0);
        plugin.initialize(SR, 4096);
        let mut left = input.clone();
        let mut right = input.clone();
        // Identical engaged state after the shared 512-block prefix.
        run_blocks(&mut plugin, &mut left[..prefix], &mut right[..prefix], 512);
        assert!(plugin.pitch_sync_engaged(), "voice path never engaged");
        let (l_rest, r_rest) = (&mut left[prefix..], &mut right[prefix..]);
        run_blocks(&mut plugin, l_rest, r_rest, block);
        (left[prefix..].to_vec(), plugin.psola_recent_onsets())
    };

    let (big, onsets_big) = render_tail(512);
    let (small, onsets_small) = render_tail(128);

    // The onset lattices coincide: same count in the log, and each of
    // the last 64 onsets lands within half a sample of its twin.
    assert_eq!(onsets_big.len(), onsets_small.len());
    let max_onset_diff = onsets_big
        .iter()
        .zip(onsets_small.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f64, f64::max);
    assert!(
        max_onset_diff < 0.5,
        "onset lattice depends on block size: max onset diff {max_onset_diff} samples"
    );

    // The rendered envelope is unchanged (10 ms window RMS within 5%
    // wherever the signal is above a tenth of its mean level).
    let win = (0.010 * SR) as usize;
    let (eb, es) = (envelope(&big, win), envelope(&small, win));
    let floor = 0.1 * eb.iter().sum::<f32>() / eb.len() as f32;
    for (i, (a, b)) in eb.iter().zip(es.iter()).enumerate() {
        if *a > floor || *b > floor {
            let rel = (a - b).abs() / a.max(*b);
            assert!(
                rel < 0.05,
                "envelope depends on block size at window {i}: {a} vs {b}"
            );
        }
    }
}

/// The audio path performs no allocation with the voice path active,
/// through an unvoiced fallback and a re-engage (tracker feed, marker
/// ring, voice pool and blend are all pre-allocated).
#[test]
fn voice_mode_does_not_allocate() {
    let mut plugin = voice_plugin(2, 200.0, 0.0);
    plugin.initialize(SR, 512);

    let block = 512usize;
    let saw_in = saw((3.5 * SR) as usize, 150.0, 0.6, 0.0, 0.0);
    let noise_in = noise(SR as usize, 0.4, 0xFEED_1082);
    // Pre-allocated processing buffers.
    let mut l = vec![0.0f32; block];
    let mut r = vec![0.0f32; block];
    let mut run_span = |plugin: &mut ResonanceGranularDelay, src: &[f32]| {
        for chunk in src.chunks_exact(block) {
            l.copy_from_slice(chunk);
            r.copy_from_slice(chunk);
            let mut outs = [OutputBuffer {
                left: &mut l[..],
                right: &mut r[..],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, block, &mut ev, None);
        }
    };

    // Warm-up: engage the voice path.
    run_span(&mut plugin, &saw_in[..SR as usize]);
    assert!(plugin.pitch_sync_engaged(), "voice path never engaged");

    let before = thread_allocs();
    run_span(&mut plugin, &saw_in[SR as usize..2 * SR as usize]); // voiced
    run_span(&mut plugin, &noise_in); // fallback transition
    run_span(&mut plugin, &saw_in[2 * SR as usize..]); // re-engage
    let after = thread_allocs();
    assert_eq!(
        after - before,
        0,
        "voice mode allocated {} times on the audio path",
        after - before
    );
}
