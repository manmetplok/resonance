//! Freeze/hold behaviour through the full plugin stack (ba todo #1075,
//! doc #252 §1): indefinite sustain from a stopped write head, a
//! bit-stable buffer while frozen (even with over-unity feedback),
//! click-free engage/resume crossfades, correct resumption of
//! streaming, and the no-allocation guarantee with freeze toggling.

use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

const SR: f32 = 48_000.0;

// --- Per-thread allocation counter for the no-allocation guard --------
// (pattern from tests/plugin.rs / resonance-dsp/tests/granular.rs).

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

/// Deterministic wet-only plugin (Sync scheduler, zero jitters).
fn freeze_plugin(time_ms: f32, feedback: f32) -> ResonanceGranularDelay {
    let plugin = ResonanceGranularDelay::new();
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_ms.set_value(time_ms);
    plugin.params.grain_size_ms.set_value(90.0);
    plugin.params.density_hz.set_value(25.0);
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
    plugin
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Largest second difference — the click/discontinuity detector from
/// tests/granulate.rs (insensitive to smooth signals).
fn max_second_difference(x: &[f32]) -> f32 {
    x.windows(3)
        .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
        .fold(0.0_f32, f32::max)
}

fn sine(frames: usize, freq: f32, amp: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| (std::f32::consts::TAU * freq * i as f32 / SR).sin() * amp)
        .collect()
}

// --- Tests ------------------------------------------------------------

/// DoD: with freeze engaged the wet output sustains indefinitely — the
/// RMS 10 s into the freeze stays within tolerance of the RMS at freeze
/// time — and the write head demonstrably stops.
#[test]
fn freeze_sustains_wet_output_indefinitely() {
    let block = 1024;
    let mut plugin = freeze_plugin(200.0, 0.0);
    plugin.initialize(SR, block as u32);

    // 2 s of live sine to fill the buffer.
    let fill = 2 * SR as usize;
    let mut left = sine(fill, 220.0, 0.8);
    let mut right = left.clone();
    run_blocks(&mut plugin, &mut left, &mut right, block);

    plugin.params.freeze.set_plain(1.0);
    // Let the engage ramp (5 ms) complete, then note the head position.
    let mut l = vec![0.0f32; SR as usize / 10];
    let mut r = vec![0.0f32; SR as usize / 10];
    run_blocks(&mut plugin, &mut l, &mut r, block);
    let head_at_freeze = plugin.write_head();

    // 10 s frozen with silent input; the wet must keep sounding.
    let held = 10 * SR as usize;
    let mut l = vec![0.0f32; held];
    let mut r = vec![0.0f32; held];
    run_blocks(&mut plugin, &mut l, &mut r, block);

    assert_eq!(
        plugin.write_head(),
        head_at_freeze,
        "write head advanced while frozen"
    );

    let early = rms(&l[..SR as usize / 2]);
    let late = rms(&l[held - SR as usize / 2..]);
    assert!(
        early > 0.05,
        "frozen wet output is silent right after freeze: rms {early}"
    );
    assert!(
        late > early * 0.5 && late < early * 2.0,
        "frozen sustain drifted after 10 s: rms at freeze {early}, after 10 s {late}"
    );
    for &x in &l {
        assert!(x.is_finite(), "non-finite sample while frozen: {x}");
    }
}

/// DoD: the buffer contents are byte-identical across the frozen span —
/// here under the worst case, 110 % Wet→Buffer feedback — and frozen
/// feedback cannot run away (writes are gated, so the loop is open).
#[test]
fn frozen_buffer_is_bit_stable_even_at_110_percent_feedback() {
    let block = 512;
    let mut plugin = freeze_plugin(200.0, 1.1);
    plugin.initialize(SR, block as u32);

    let fill = 2 * SR as usize;
    let mut left = sine(fill, 220.0, 0.8);
    let mut right = left.clone();
    run_blocks(&mut plugin, &mut left, &mut right, block);

    plugin.params.freeze.set_plain(1.0);
    // Complete the engage ramp, then snapshot the buffer bits.
    let mut l = vec![0.0f32; SR as usize / 10];
    let mut r = vec![0.0f32; SR as usize / 10];
    run_blocks(&mut plugin, &mut l, &mut r, block);
    let snap_l: Vec<u32> = plugin.ring_l().iter().map(|x| x.to_bits()).collect();
    let snap_r: Vec<u32> = plugin.ring_r().iter().map(|x| x.to_bits()).collect();

    // 10 s frozen with the input still playing (it must be ignored).
    let held = 10 * SR as usize;
    let mut l = sine(held, 330.0, 0.8);
    let mut r = l.clone();
    run_blocks(&mut plugin, &mut l, &mut r, block);

    let now_l: Vec<u32> = plugin.ring_l().iter().map(|x| x.to_bits()).collect();
    let now_r: Vec<u32> = plugin.ring_r().iter().map(|x| x.to_bits()).collect();
    assert!(
        snap_l == now_l && snap_r == now_r,
        "frozen buffer was modified (first differing L index: {:?})",
        snap_l.iter().zip(now_l.iter()).position(|(a, b)| a != b)
    );

    // Frozen buffer + over-unity feedback must not run away: the loop
    // is open (no writes), so the wet just re-reads static content.
    let mut peak = 0.0f32;
    for &x in &l {
        assert!(x.is_finite(), "non-finite sample while frozen: {x}");
        peak = peak.max(x.abs());
    }
    assert!(
        peak < 2.0,
        "frozen 110% feedback exceeded +6 dBFS: peak {peak}"
    );
}

/// DoD: toggling freeze on and then off during a steady sine produces
/// no discontinuity above the click threshold at either transition —
/// the engage/resume write-gain crossfade keeps the buffer smooth, so
/// the granulated output stays smooth too.
#[test]
fn freeze_toggle_during_sine_is_click_free() {
    let block = 512;
    let mut plugin = freeze_plugin(200.0, 0.0);
    plugin.initialize(SR, block as u32);

    // Pre-lap the ring (4 s => 2^18 samples ≈ 5.46 s) with an unrelated
    // 130 Hz sine so the content *ahead* of the stopped head is
    // non-zero: the engage ramp must morph the recorded stream into
    // that lap-old material (and resume back out of it) without a
    // splice — the hardest case for the crossfade.
    let lap = ((4.0 * SR) as usize + 1).next_power_of_two() + SR as usize;
    let mut l = sine(lap, 130.0, 0.9);
    let mut r = l.clone();
    run_blocks(&mut plugin, &mut l, &mut r, block);

    // 50 Hz keeps the source's own second difference tiny (same trick
    // as tests/granulate.rs), so the detector isolates freeze
    // artifacts. Render continuously, toggling freeze on at 1 s and
    // off at 2 s, block-aligned.
    let frames = 3 * SR as usize;
    let mut left = sine(frames, 50.0, 0.9);
    let mut right = left.clone();

    let mut pos = 0;
    let mut engaged = false;
    let mut resumed = false;
    while pos < frames {
        if !engaged && pos >= SR as usize {
            plugin.params.freeze.set_plain(1.0);
            engaged = true;
        }
        if !resumed && pos >= 2 * SR as usize {
            plugin.params.freeze.set_plain(0.0);
            resumed = true;
        }
        let n = (frames - pos).min(block);
        let mut outs = [OutputBuffer {
            left: &mut left[pos..pos + n],
            right: &mut right[pos..pos + n],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        pos += n;
    }

    // Skip the initial cloud fill; cover both transitions and the
    // post-resume settling.
    let steady = &left[(0.4 * SR) as usize..];
    assert!(
        rms(steady) > 0.05,
        "wet bus unexpectedly quiet: rms {}",
        rms(steady)
    );
    let d2 = max_second_difference(steady);
    assert!(
        d2 < 0.02,
        "freeze engage/resume clicked: max d2 {d2}"
    );
}

/// After resume the plugin streams again: the write head advances and
/// newly arriving material re-emerges at the configured delay.
#[test]
fn resume_restores_streaming_at_the_delay() {
    let block = 512;
    let mut plugin = freeze_plugin(250.0, 0.0);
    plugin.initialize(SR, block as u32);

    // Fill, freeze for 1 s, then resume with silence.
    let mut l = sine(SR as usize, 220.0, 0.8);
    let mut r = l.clone();
    run_blocks(&mut plugin, &mut l, &mut r, block);
    plugin.params.freeze.set_plain(1.0);
    let mut l = vec![0.0f32; SR as usize];
    let mut r = vec![0.0f32; SR as usize];
    run_blocks(&mut plugin, &mut l, &mut r, block);
    plugin.params.freeze.set_plain(0.0);
    let head_at_resume = plugin.write_head();

    // Let the old content flush past the 250 ms delay window, then send
    // an impulse and expect its granulated echo one delay later.
    let mut l = vec![0.0f32; SR as usize];
    let mut r = vec![0.0f32; SR as usize];
    run_blocks(&mut plugin, &mut l, &mut r, block);
    assert!(
        plugin.write_head() > head_at_resume,
        "write head did not resume advancing"
    );

    let frames = SR as usize;
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    l[0] = 1.0;
    r[0] = 1.0;
    run_blocks(&mut plugin, &mut l, &mut r, block);

    let delay_samples = (0.250 * SR) as usize;
    let at: f32 = l[delay_samples - 64..delay_samples + 256]
        .iter()
        .map(|x| x.abs())
        .sum();
    assert!(
        at > 1e-3,
        "no granulated echo at the delay after resume, got {at}"
    );
}

/// The audio path stays allocation-free across freeze engage, the held
/// span and resume — with feedback active too.
#[test]
fn audio_path_with_freeze_toggling_does_not_allocate() {
    let mut plugin = freeze_plugin(200.0, 1.0);
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
    plugin.params.freeze.set_plain(1.0); // engage
    run_blocks(&mut plugin, &mut left, &mut right, block);
    plugin.params.freeze.set_plain(0.0); // resume
    run_blocks(&mut plugin, &mut left, &mut right, block);
    let after = thread_allocs();
    assert_eq!(
        after - before,
        0,
        "freeze path allocated {} times on the audio path",
        after - before
    );
}
