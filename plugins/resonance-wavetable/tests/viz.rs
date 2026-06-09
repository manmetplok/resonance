use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use resonance_wavetable::viz::{ScopeCollector, WavetableVizState, SCOPE_FRAMES};

/// Two-thread stress test: one thread stores scalars and publishes scope
/// frames at audio-like rates; another reads snapshots at ~60 Hz. Verifies
/// no tearing in the scope buffer (seq-lock invariant) and that the
/// sample count is monotonic.
#[test]
fn viz_bridge_no_tearing() {
    let viz = Arc::new(WavetableVizState::new());
    let done = Arc::new(AtomicBool::new(false));

    let writer = {
        let viz = viz.clone();
        let done = done.clone();
        thread::spawn(move || {
            let mut collector = ScopeCollector::new();
            let mut t = 0u32;
            while !done.load(Ordering::Acquire) {
                // Fill a full scope with a recognisable "ramp"
                // pattern: each frame is (L=t, R=-t).
                for _ in 0..SCOPE_FRAMES {
                    let v = (t as f32) * 0.001;
                    collector.push(v, -v);
                    t = t.wrapping_add(1);
                }
                // Writer-side scalars.
                viz.store_lfo_phase(0, (t as f32 * 0.0001) % 1.0);
                viz.store_env_amp((t as f32 * 0.0002) % 1.0, 1);
                collector.publish(&viz);
                thread::sleep(Duration::from_micros(500));
            }
        })
    };

    let reader = {
        let viz = viz.clone();
        let done = done.clone();
        thread::spawn(move || -> (bool, u32) {
            let mut last_count = 0u32;
            let mut monotonic = true;
            let deadline = Instant::now() + Duration::from_millis(500);
            while Instant::now() < deadline && !done.load(Ordering::Acquire) {
                let snap = viz.read_snapshot();
                // Tear check: every stereo pair (L, R) must satisfy R == -L
                // because the writer stores them in that relationship.
                for pair in snap.scope_samples.chunks_exact(2) {
                    if (pair[0] + pair[1]).abs() > 1e-6 {
                        return (false, snap.scope_sample_count);
                    }
                }
                if snap.scope_sample_count < last_count {
                    monotonic = false;
                }
                last_count = snap.scope_sample_count;
                thread::sleep(Duration::from_micros(16_000));
            }
            (monotonic, last_count)
        })
    };

    thread::sleep(Duration::from_millis(500));
    done.store(true, Ordering::Release);
    writer.join().unwrap();
    let (monotonic, final_count) = reader.join().unwrap();

    assert!(monotonic, "scope_sample_count regressed");
    assert!(
        final_count > 0,
        "reader saw zero samples — writer never published"
    );
}

/// `publish` must present the ring in chronological order (oldest first)
/// even when the collector's write position has wrapped mid-ring — the
/// rotation now happens inside `publish_scope`'s store pass instead of via
/// an intermediate ordered copy, and this pins the equivalence.
#[test]
fn publish_orders_wrapped_ring_chronologically() {
    let viz = WavetableVizState::new();
    let mut collector = ScopeCollector::new();

    // Push 1.5 rings worth of frames so the write cursor wraps and sits
    // mid-buffer. Frame i is (L=i, R=-i).
    let total = SCOPE_FRAMES + SCOPE_FRAMES / 2;
    for i in 0..total {
        collector.push(i as f32, -(i as f32));
    }
    collector.publish(&viz);

    let snap = viz.read_snapshot();
    // The ring retains the most recent SCOPE_FRAMES frames:
    // [total - SCOPE_FRAMES, total).
    let first = (total - SCOPE_FRAMES) as f32;
    for (frame, pair) in snap.scope_samples.chunks_exact(2).enumerate() {
        let expect = first + frame as f32;
        assert_eq!(pair[0], expect, "left sample out of order at {frame}");
        assert_eq!(pair[1], -expect, "right sample out of order at {frame}");
    }
    assert_eq!(snap.scope_sample_count, total as u32);
}

/// Publishing with the write cursor at 0 (exactly full ring, no wrap) is
/// the degenerate rotation: output equals the ring as-is.
#[test]
fn publish_with_unwrapped_ring_is_identity() {
    let viz = WavetableVizState::new();
    let mut collector = ScopeCollector::new();

    for i in 0..SCOPE_FRAMES {
        collector.push(i as f32, i as f32 + 0.5);
    }
    collector.publish(&viz);

    let snap = viz.read_snapshot();
    for (frame, pair) in snap.scope_samples.chunks_exact(2).enumerate() {
        assert_eq!(pair[0], frame as f32);
        assert_eq!(pair[1], frame as f32 + 0.5);
    }
}
