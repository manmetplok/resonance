//! Delay-time change modes (ba todo #1076, doc #252 §5): stepping Time
//! under Fade, Repitch and Per-Grain. Fade completes through a ~20 ms
//! dual-tap swap with no click and no pitch excursion; Repitch slews
//! the effective delay monotonically with the tape-style rate swoop
//! (proportional to the time delta, settling back to unity); Per-Grain
//! (default) leaves in-flight grains untouched while new grains latch
//! the new time. The audio path stays allocation-free across time-mode
//! changes mid-run.

use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

const SR: f32 = 48_000.0;

// --- Per-thread allocation counter for the no-allocation guard --------
// (pattern from tests/plugin.rs).

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

/// Deterministic wet-only plugin under the given time mode: free-running
/// time, Sync scheduler, no jitters, no feedback (unless overridden).
fn mode_plugin(time_mode: i32, time_ms: f32, grain_ms: f32, density_hz: f32) -> ResonanceGranularDelay {
    let plugin = ResonanceGranularDelay::new();
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_mode.set_value(time_mode);
    plugin.params.time_ms.set_value(time_ms);
    plugin.params.grain_size_ms.set_value(grain_ms);
    plugin.params.density_hz.set_value(density_hz);
    plugin.params.scheduler.set_value(0); // Sync (deterministic)
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

/// Render `input` (both channels), stepping `time_ms` to `new_ms` at
/// sample `change_at`. Returns the left output and the plugin for
/// introspection.
fn render_step(
    time_mode: i32,
    old_ms: f32,
    new_ms: f32,
    grain_ms: f32,
    density_hz: f32,
    input: &[f32],
    change_at: usize,
) -> (Vec<f32>, ResonanceGranularDelay) {
    let mut plugin = mode_plugin(time_mode, old_ms, grain_ms, density_hz);
    plugin.initialize(SR, 4096);
    let mut left = input.to_vec();
    let mut right = input.to_vec();
    run_blocks(&mut plugin, &mut left[..change_at], &mut right[..change_at], 512);
    plugin.params.time_ms.set_value(new_ms);
    let (l_rest, r_rest) = (&mut left[change_at..], &mut right[change_at..]);
    run_blocks(&mut plugin, l_rest, r_rest, 512);
    (left, plugin)
}

/// Largest second difference — click/discontinuity detector (same as
/// tests/granulate.rs).
fn max_second_difference(x: &[f32]) -> f32 {
    x.windows(3)
        .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
        .fold(0.0_f32, f32::max)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Zero crossings with a small amplitude floor so near-silent spans
/// (e.g. the Fade dip) cannot fabricate sign wobble.
fn zero_crossings(x: &[f32]) -> usize {
    let mut count = 0;
    let mut last = 0.0f32;
    for &v in x {
        if v.abs() < 1e-4 {
            continue;
        }
        if last != 0.0 && (v > 0.0) != (last > 0.0) {
            count += 1;
        }
        last = v;
    }
    count
}

fn sine(frames: usize, freq: f32, amp: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| (std::f32::consts::TAU * freq * i as f32 / SR).sin() * amp)
        .collect()
}

// --- Tests ------------------------------------------------------------

/// Fade retires the old tap within ~25 ms while Per-Grain lets
/// in-flight grains finish at their old origin. Discriminator: the old
/// tap (1200 ms) reads loud sine, the new tap (100 ms) reads silence,
/// so post-change energy is exactly the old tap's survival. Both modes
/// stay click-free through the step.
#[test]
fn fade_completes_fast_while_per_grain_lets_old_grains_finish() {
    let frames = (2.5 * SR) as usize;
    let change_at = (2.0 * SR) as usize;
    let mut input = sine(frames, 50.0, 0.9);
    // Silence after 1.5 s: the new 100 ms tap reads only silence.
    for v in input[(1.5 * SR) as usize..].iter_mut() {
        *v = 0.0;
    }

    // Old-tap survival window: 40..120 ms after the change (past the
    // ~25 ms Fade transition, well inside the 200 ms grain length).
    let w0 = change_at + (0.040 * SR) as usize;
    let w1 = change_at + (0.120 * SR) as usize;
    // Click window across the step.
    let c0 = change_at - (0.100 * SR) as usize;
    let c1 = change_at + (0.100 * SR) as usize;

    // Per-Grain (mode 2, the default).
    let (out_pg, plugin_pg) = render_step(2, 1200.0, 100.0, 200.0, 25.0, &input, change_at);
    let rms_pg = rms(&out_pg[w0..w1]);
    assert!(
        rms_pg > 0.02,
        "per-grain retired in-flight grains early: rms {rms_pg}"
    );
    let d2 = max_second_difference(&out_pg[c0..c1]);
    assert!(d2 < 0.02, "per-grain time step clicked: max d2 {d2}");
    // In-flight grains are unaffected: every live grain still runs at
    // exactly unity rate (no retuning, no restart).
    for rate in plugin_pg.active_rates() {
        assert!(
            (rate - 1.0).abs() < 1e-9,
            "per-grain change disturbed a grain rate: {rate}"
        );
    }
    // ... and the effective tap position jumped immediately.
    let eff = plugin_pg.effective_delay_seconds();
    assert!(
        (eff - 0.1).abs() < 1e-6,
        "per-grain effective delay did not jump: {eff}"
    );

    // Fade (mode 0): old-tap energy must be gone shortly after ~20 ms.
    let (out_fade, _) = render_step(0, 1200.0, 100.0, 200.0, 25.0, &input, change_at);
    let rms_fade = rms(&out_fade[w0..w1]);
    assert!(
        rms_fade < 1e-3,
        "fade transition did not complete within ~25 ms: rms {rms_fade}"
    );
    let d2 = max_second_difference(&out_fade[c0..c1]);
    assert!(d2 < 0.02, "fade time step clicked: max d2 {d2}");
}

/// Fade on a continuous sine: the step is click-free and pitch-stable —
/// the zero-crossing rate through the transition stays at the source
/// rate (no repitch swoop), unlike Repitch on the identical scenario.
#[test]
fn fade_step_is_click_free_and_pitch_stable() {
    let frames = (3.0 * SR) as usize;
    let change_at = (2.0 * SR) as usize;
    let input = sine(frames, 220.0, 0.8);

    let (out, _) = render_step(0, 300.0, 400.0, 60.0, 40.0, &input, change_at);

    // Self-calibrated nominal ZCR from the pre-change steady cloud.
    let win = (0.25 * SR) as usize;
    let nominal = zero_crossings(&out[change_at - 2 * win..change_at - win]);
    let through = zero_crossings(&out[change_at..change_at + win]);
    assert!(
        through as f32 >= 0.85 * nominal as f32 && through as f32 <= 1.15 * nominal as f32,
        "fade transition moved the zero-crossing rate: {through} vs nominal {nominal}"
    );
    let d2 = max_second_difference(
        &out[change_at - (0.1 * SR) as usize..change_at + (0.3 * SR) as usize],
    );
    assert!(d2 < 0.015, "fade transition clicked: max d2 {d2}");
}

/// Repitch on the identical scenario: the transient rate excursion IS
/// present — the zero-crossing rate through the transition drops well
/// below nominal (delay grows => tape slows => pitch down) — and the
/// effective delay glides monotonically to the target with no jump,
/// after which all grain rates settle back to exactly unity.
#[test]
fn repitch_swoops_and_glides_monotonically_to_the_target() {
    let frames = (2.0 * SR) as usize;
    let change_at = frames; // change applied after this prefix
    let input = sine(frames, 220.0, 0.8);

    let mut plugin = mode_plugin(1, 300.0, 60.0, 40.0);
    plugin.initialize(SR, 4096);
    let mut left = input.clone();
    let mut right = input.clone();
    run_blocks(&mut plugin, &mut left, &mut right, 512);
    let _ = change_at;

    plugin.params.time_ms.set_value(400.0);
    // Drive 2 s block by block, tracking the effective delay per block.
    let post = (2.0 * SR) as usize;
    let mut pl = sine(post, 220.0, 0.8);
    let mut pr = pl.clone();
    let mut effs = Vec::new();
    let mut pos = 0;
    while pos < post {
        let n = (post - pos).min(512);
        {
            let mut outs = [OutputBuffer {
                left: &mut pl[pos..pos + n],
                right: &mut pr[pos..pos + n],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, n, &mut ev, None);
        }
        effs.push(plugin.effective_delay_seconds());
        pos += n;
    }

    // Monotonic glide, bounded steps, no overshoot, exact settle.
    for pair in effs.windows(2) {
        assert!(
            pair[1] >= pair[0] - 1e-9,
            "effective delay moved backwards: {} -> {}",
            pair[0],
            pair[1]
        );
        assert!(
            pair[1] - pair[0] < 0.02,
            "effective delay jumped: {} -> {}",
            pair[0],
            pair[1]
        );
    }
    for &e in &effs {
        assert!(
            (0.3 - 1e-6..=0.4 + 1e-6).contains(&e),
            "effective delay left the glide range: {e}"
        );
    }
    assert!(
        (effs[0] - 0.3).abs() < 0.02,
        "first block already jumped: {}",
        effs[0]
    );
    let last = *effs.last().unwrap();
    assert!(
        (last - 0.4).abs() < 1e-6,
        "repitch never settled on the target: {last}"
    );

    // The swoop is audible in the zero-crossing rate...
    let win = (0.25 * SR) as usize;
    let nominal = zero_crossings(&left[left.len() - 2 * win..left.len() - win]);
    let through = zero_crossings(&pl[..win]);
    assert!(
        (through as f32) < 0.85 * nominal as f32,
        "repitch produced no rate excursion: {through} vs nominal {nominal}"
    );
    // ... and every grain has settled back to exactly unity rate.
    for rate in plugin.active_rates() {
        assert!(
            (rate - 1.0).abs() < 1e-6,
            "grain rate never settled back to unity: {rate}"
        );
    }
}

/// The Repitch rate excursion is proportional to the time delta: a
/// 10 ms step swoops twice as far as a 5 ms step (one-pole slew: the
/// initial d(delay)/dt scales with the delta).
#[test]
fn repitch_rate_excursion_scales_with_the_time_delta() {
    let excursion = |new_ms: f32| -> f64 {
        let pre = (1.0 * SR) as usize;
        let input = sine(pre, 220.0, 0.8);
        let mut plugin = mode_plugin(1, 300.0, 60.0, 40.0);
        plugin.initialize(SR, 4096);
        let mut left = input.clone();
        let mut right = input.clone();
        run_blocks(&mut plugin, &mut left, &mut right, 512);

        plugin.params.time_ms.set_value(new_ms);
        // Track the deepest grain rate over the first ~85 ms of slew
        // (several Sync onsets at 40 grains/s).
        let mut min_rate = f64::INFINITY;
        let mut l = vec![0.0f32; 512];
        let mut r = vec![0.0f32; 512];
        for _ in 0..8 {
            for (i, v) in l.iter_mut().enumerate() {
                *v = 0.5 * (i as f32).sin();
            }
            r.copy_from_slice(&l);
            let mut outs = [OutputBuffer {
                left: &mut l[..],
                right: &mut r[..],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, 512, &mut ev, None);
            for rate in plugin.active_rates() {
                min_rate = min_rate.min(rate);
            }
        }
        1.0 - min_rate
    };

    let exc_5 = excursion(305.0);
    let exc_10 = excursion(310.0);
    assert!(
        (0.02..0.08).contains(&exc_5),
        "5 ms delta excursion out of range: {exc_5}"
    );
    let ratio = exc_10 / exc_5;
    assert!(
        (1.8..2.2).contains(&ratio),
        "excursion not proportional to delta: {exc_5} vs {exc_10} (ratio {ratio})"
    );
}

/// With a constant delay the three modes render bit-identically — the
/// mode machinery only engages on an actual time change, so the default
/// Per-Grain path (and every existing render) is untouched.
#[test]
fn constant_delay_renders_identically_across_time_modes() {
    let frames = SR as usize;
    let input = sine(frames, 110.0, 0.7);
    let render = |mode: i32| -> Vec<f32> {
        let mut plugin = mode_plugin(mode, 250.0, 90.0, 25.0);
        plugin.initialize(SR, 4096);
        let mut left = input.clone();
        let mut right = input.clone();
        run_blocks(&mut plugin, &mut left, &mut right, 512);
        left
    };
    let fade = render(0);
    let repitch = render(1);
    let per_grain = render(2);
    assert_eq!(fade, per_grain, "fade diverged with a constant delay");
    assert_eq!(repitch, per_grain, "repitch diverged with a constant delay");
}

/// The Output-only recirculation read tap follows the time mode too:
/// stepping Time under Fade and Repitch with clean repeats engaged
/// stays finite and click-free (the tap glides under Repitch and is
/// gain-masked through the Fade swap).
#[test]
fn output_only_route_follows_time_modes_without_clicks() {
    for mode in [0, 1] {
        let frames = (3.5 * SR) as usize;
        let change_at = (2.0 * SR) as usize;
        let input = sine(frames, 50.0, 0.8);
        let mut plugin = mode_plugin(mode, 300.0, 90.0, 25.0);
        plugin.params.fb_route.set_value(1); // Output-only
        plugin.params.feedback.set_value(0.5);
        plugin.initialize(SR, 4096);
        let mut left = input.clone();
        let mut right = input.clone();
        run_blocks(&mut plugin, &mut left[..change_at], &mut right[..change_at], 512);
        plugin.params.time_ms.set_value(150.0);
        let (l_rest, r_rest) = (&mut left[change_at..], &mut right[change_at..]);
        run_blocks(&mut plugin, l_rest, r_rest, 512);

        for &x in left.iter() {
            assert!(x.is_finite(), "mode {mode}: non-finite sample {x}");
        }
        let d2 = max_second_difference(
            &left[change_at - (0.1 * SR) as usize..change_at + (0.6 * SR) as usize],
        );
        assert!(
            d2 < 0.03,
            "mode {mode}: output-only recirc tap clicked on the step: max d2 {d2}"
        );
    }
}

/// Per-Grain with the Output-only route: stepping Time must not click
/// in the recirculating feedback tail. Per-Grain moves the effective
/// delay instantly, so without the fade-to-silence swap on the recirc
/// read tap the tap position jumps across a block boundary and the
/// repeats crack (the regression this pins). The 300 -> 155 ms step is
/// 7.25 periods of the 50 Hz source, so the jump lands a quarter-cycle
/// out of phase — maximally audible on the old integer tap.
#[test]
fn per_grain_output_only_time_step_does_not_click() {
    let frames = (3.5 * SR) as usize;
    let change_at = (2.0 * SR) as usize;
    let input = sine(frames, 50.0, 0.8);
    let mut plugin = mode_plugin(2, 300.0, 90.0, 25.0);
    plugin.params.fb_route.set_value(1); // Output-only
    plugin.params.feedback.set_value(0.6);
    plugin.initialize(SR, 4096);
    let mut left = input.clone();
    let mut right = input.clone();
    run_blocks(&mut plugin, &mut left[..change_at], &mut right[..change_at], 512);
    plugin.params.time_ms.set_value(155.0);
    let (l_rest, r_rest) = (&mut left[change_at..], &mut right[change_at..]);
    run_blocks(&mut plugin, l_rest, r_rest, 512);

    for &x in left.iter() {
        assert!(x.is_finite(), "non-finite sample {x}");
    }
    // The recirc tap still jumped immediately in the effective-delay
    // sense (Per-Grain semantics are untouched)...
    let eff = plugin.effective_delay_seconds();
    assert!(
        (eff - 0.155).abs() < 1e-6,
        "per-grain effective delay did not jump: {eff}"
    );
    // ... but the feedback tail through the step stays as smooth as the
    // steady-state cloud around it (the old integer tap cracked here at
    // an order of magnitude above steady state).
    let steady = max_second_difference(&left[change_at - (0.5 * SR) as usize..change_at]);
    let through = max_second_difference(&left[change_at..change_at + (0.2 * SR) as usize]);
    assert!(
        through < 3.0 * steady.max(1e-3),
        "per-grain output-only recirc tap clicked on the step: \
         max d2 {through} vs steady {steady}"
    );
}

/// With a constant delay the Output-only recirculation renders
/// bit-identically across the three time modes — the recirc-tap swap
/// machine only engages on an actual Per-Grain time change, so every
/// static clean-repeats render keeps its exact bits.
#[test]
fn constant_delay_output_only_renders_identically_across_time_modes() {
    let frames = 2 * SR as usize;
    let input = sine(frames, 110.0, 0.7);
    let render = |mode: i32| -> Vec<f32> {
        let mut plugin = mode_plugin(mode, 250.0, 90.0, 25.0);
        plugin.params.fb_route.set_value(1); // Output-only
        plugin.params.feedback.set_value(0.5);
        plugin.initialize(SR, 4096);
        let mut left = input.clone();
        let mut right = input.clone();
        run_blocks(&mut plugin, &mut left, &mut right, 512);
        left
    };
    let fade = render(0);
    let repitch = render(1);
    let per_grain = render(2);
    assert_eq!(fade, per_grain, "fade diverged with a constant delay");
    assert_eq!(repitch, per_grain, "repitch diverged with a constant delay");
}

/// The audio path performs no allocation across repeated time steps in
/// Fade mode, a mid-run switch to Repitch and further steps (fader,
/// slew state and per-sample buffers are all pre-allocated).
#[test]
fn time_mode_changes_do_not_allocate() {
    let mut plugin = mode_plugin(0, 150.0, 90.0, 30.0);
    plugin.initialize(SR, 512);

    let block = 512usize;
    // Pre-allocated outside the counted region; refilled per run.
    let mut l = vec![0.2f32; block];
    let mut r = vec![0.2f32; block];
    let mut seconds = |plugin: &mut ResonanceGranularDelay, s: f32| {
        let n = (s * SR) as usize / block;
        for _ in 0..n {
            l.fill(0.2);
            r.fill(0.2);
            let mut outs = [OutputBuffer {
                left: &mut l[..],
                right: &mut r[..],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, block, &mut ev, None);
        }
    };

    // Warm-up, including one Fade swap so both legs have run.
    seconds(&mut plugin, 0.2);
    plugin.params.time_ms.set_value(350.0);
    seconds(&mut plugin, 0.3);

    let before = thread_allocs();
    plugin.params.time_ms.set_value(150.0);
    seconds(&mut plugin, 0.25);
    plugin.params.time_ms.set_value(350.0);
    seconds(&mut plugin, 0.25);
    plugin.params.time_mode.set_value(1); // Repitch, mid-run
    plugin.params.time_ms.set_value(120.0);
    seconds(&mut plugin, 0.25);
    plugin.params.time_ms.set_value(400.0);
    seconds(&mut plugin, 0.5);
    plugin.params.time_mode.set_value(2); // back to Per-Grain
    seconds(&mut plugin, 0.25);
    // Per-Grain + Output-only time steps ride the recirc-tap swap
    // machine, which is likewise pre-allocated.
    plugin.params.fb_route.set_value(1);
    plugin.params.feedback.set_value(0.5);
    seconds(&mut plugin, 0.25);
    plugin.params.time_ms.set_value(250.0);
    seconds(&mut plugin, 0.25);
    let after = thread_allocs();
    assert_eq!(
        after - before,
        0,
        "time-mode processing allocated {} times",
        after - before
    );
}
