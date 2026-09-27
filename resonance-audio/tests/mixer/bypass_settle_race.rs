//! FU-B4a: `BypassFade::set_bypassed_settled` is meant to land the fade on
//! its end state with no transition — the path `apply_bypass_request`
//! takes when it believes "nothing is rendering" (transport stopped, no
//! monitoring). That belief can race a block that *is* rendering (a Play
//! that is just starting, or the monitor path), and `set_bypassed_settled`
//! used to write its target and its position as two separate atomics:
//!
//! ```text
//! self.bypassed.store(v, Relaxed);   // 1
//! self.pos.store(0 or MAX, Relaxed); // 2
//! ```
//!
//! A block whose `stage()` call lands between 1 and 2 observes a torn
//! pair — the new target with the *old* position — which does not equal
//! the settled goal, so `stage()` falls through to its normal fade branch
//! and starts crossfading over [`BYPASS_FADE_MS`] instead of landing
//! instantly. Since this test only ever calls `set_bypassed_settled` (never
//! the crossfading `set_bypassed`), there is no legitimate fade in flight
//! at all: every `stage()` call must resolve to [`FadeStage::Wet`] or
//! [`FadeStage::Dry`], never [`FadeStage::Fade`]. A `Fade` shows up only
//! when a block observed a torn pair.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use resonance_audio::test_support::{BypassFade, FadeStage};

const SR: u32 = 48_000;
const FRAMES: usize = 128;
const ITERATIONS: u64 = 300_000;

#[test]
fn settled_bypass_never_tears_against_a_concurrent_render() {
    let fade = Arc::new(BypassFade::new());
    let stop = Arc::new(AtomicBool::new(false));
    let bad_fades = Arc::new(AtomicU64::new(0));
    let stages_seen = Arc::new(AtomicU64::new(0));

    let settler = {
        let fade = Arc::clone(&fade);
        let stop = Arc::clone(&stop);
        let stages_seen = Arc::clone(&stages_seen);
        std::thread::spawn(move || {
            // Start toggling only once the renderer is running: under load
            // the scheduler can otherwise finish every toggle before the
            // renderer thread first runs, and the race never happens.
            while stages_seen.load(Ordering::Relaxed) == 0 {
                std::hint::spin_loop();
            }
            let mut toggles = 0u64;
            let mut v = false;
            while toggles < ITERATIONS {
                v = !v;
                fade.set_bypassed_settled(v);
                toggles += 1;
            }
            stop.store(true, Ordering::Release);
            toggles
        })
    };

    let renderer = {
        let fade = Arc::clone(&fade);
        let stop = Arc::clone(&stop);
        let bad_fades = Arc::clone(&bad_fades);
        let stages_seen = Arc::clone(&stages_seen);
        std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let stage = fade.stage(SR, FRAMES, true);
                stages_seen.fetch_add(1, Ordering::Relaxed);
                if matches!(stage, FadeStage::Fade { .. }) {
                    bad_fades.fetch_add(1, Ordering::Relaxed);
                }
            }
        })
    };

    let toggles = settler.join().expect("settler");
    renderer.join().expect("renderer");

    assert!(toggles > 0, "the settler actually ran");
    assert!(
        stages_seen.load(Ordering::Relaxed) > 0,
        "the renderer actually raced the settler"
    );
    assert_eq!(
        bad_fades.load(Ordering::Relaxed),
        0,
        "a block observed a torn (target, position) pair from set_bypassed_settled \
         and started a spurious crossfade instead of landing instantly"
    );
}
