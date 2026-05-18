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
