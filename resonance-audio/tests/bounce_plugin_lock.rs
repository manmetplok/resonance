//! Regression test for the bounce thread's per-plugin lock acquisition.
//!
//! Previously `engine/bounce/render.rs` called `Mutex::lock()` per
//! plugin per chunk, which forced the audio thread's `try_lock` to
//! fail for the duration of the bounce thread's `process()` — audible
//! as glitches during a bounce. The fix routes every bounce-side
//! plugin lock through `try_lock_with_backoff`, which is non-blocking:
//! it spins briefly, then sleeps in micro-bursts until the lock is
//! free.
//!
//! These tests use `try_lock_with_backoff` against a plain
//! `parking_lot::Mutex<u32>` so we can simulate contention without
//! spinning up a CLAP plugin.
//!
//! Determinism (code review FU-F2d): the earlier version ordered its
//! threads with fixed sleeps — "1 ms is enough for the holder to have
//! taken the lock", "the audio thread will have polled again before
//! `stop` lands" — and failed roughly two runs in five on a loaded
//! machine. Every ordering below is now an explicit hand-off (a barrier
//! or a flag the other side waits on) and every "the helper waited"
//! claim is proven by a value written under the lock, not by a clock.
//! The only durations left are generous wedge deadlines.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use resonance_audio::test_support::try_lock_with_backoff;

/// Upper bound on any wait in these tests: a wedge, not jitter.
const WEDGE: Duration = Duration::from_secs(10);

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < WEDGE, "timed out waiting for {what}");
        thread::yield_now();
    }
}

#[test]
fn uncontended_lock_returns_immediately() {
    // Sanity: with no other holder, the helper must take the fast
    // `try_lock` path and never sleep. The helper's first sleep is
    // 100 µs, so a call that took the slow path costs at least that;
    // the fast path is nanoseconds. The minimum over many calls is
    // immune to a preemption landing on any one of them.
    let m = Mutex::new(0u32);
    let mut fastest = Duration::MAX;
    for _ in 0..200 {
        let start = Instant::now();
        let g = try_lock_with_backoff(&m);
        fastest = fastest.min(start.elapsed());
        drop(g);
    }
    assert!(
        fastest < Duration::from_micros(100),
        "fastest uncontended acquisition took {fastest:?}; the helper slept on an unheld lock",
    );
}

#[test]
fn yields_lock_to_concurrent_try_lock_holder() {
    // The behaviour the original bug regressed: while the bounce-side
    // helper waits, an audio-thread-style `try_lock` from another
    // thread MUST be able to grab the mutex.
    //
    // The test thread holds the mutex; the "bounce" thread is inside
    // `try_lock_with_backoff`; the "audio" thread polls `try_lock`. Once
    // both workers have reported in, the test thread releases. With the
    // old blocking `mutex.lock()` the audio thread's `try_lock` could
    // starve behind the queued bounce thread; with the helper it wins
    // as soon as the mutex is free — before or after the bounce thread's
    // brief turn, which the scheduler decides, so the assertion is that
    // it wins at all, and that the bounce thread also completes.

    let m = Arc::new(Mutex::new(0u32));
    let audio_acquisitions = Arc::new(AtomicU64::new(0));
    let audio_attempts = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    // Both workers + the test thread meet here once the workers are at
    // their loops; the bounce thread's helper call follows immediately.
    let ready = Arc::new(Barrier::new(3));

    // Hold the lock so the bounce thread cannot get it on first try.
    let initial_guard = m.lock();

    let bounce = {
        let m = Arc::clone(&m);
        let ready = Arc::clone(&ready);
        thread::spawn(move || {
            ready.wait();
            let g = try_lock_with_backoff(&m);
            // Touch the value so the optimiser can't elide the guard.
            *g
        })
    };

    let audio = {
        let m = Arc::clone(&m);
        let ready = Arc::clone(&ready);
        let acquisitions = Arc::clone(&audio_acquisitions);
        let attempts = Arc::clone(&audio_attempts);
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            ready.wait();
            while !stop.load(Ordering::Relaxed) {
                attempts.fetch_add(1, Ordering::Relaxed);
                if let Some(_g) = m.try_lock() {
                    acquisitions.fetch_add(1, Ordering::Relaxed);
                    // Tiny hold so the bounce thread also gets a chance.
                    thread::sleep(Duration::from_micros(50));
                }
                thread::yield_now();
            }
        })
    };

    ready.wait();
    // The bounce thread called the helper the instant the barrier
    // opened and cannot have acquired (we hold the lock); the audio
    // thread is polling. Make sure it has actually failed at least once
    // against our guard before we let go.
    wait_until("the audio thread's first attempt", || {
        audio_attempts.load(Ordering::Relaxed) > 0
    });
    assert_eq!(audio_acquisitions.load(Ordering::Relaxed), 0);

    drop(initial_guard);

    // Both must make progress: the helper eventually wins (it retries
    // forever), and the audio thread's `try_lock` must succeed — its
    // loop keeps polling until we say stop, so this is a deadline on a
    // wedge, not a race against `stop`.
    let _value = bounce.join().expect("bounce thread panicked");
    wait_until("an audio-thread acquisition", || {
        audio_acquisitions.load(Ordering::Relaxed) > 0
    });
    stop.store(true, Ordering::Relaxed);
    audio.join().expect("audio thread panicked");
}

#[test]
fn bounce_helper_does_not_block_long_audio_holder() {
    // While another thread holds the mutex for a "long" time (well past
    // the helper's first few back-off intervals), the helper must not
    // panic, deadlock, or return before the holder releases.
    //
    // Proof of "waited": the holder rewrites the value just before it
    // releases; the helper's guard must read the rewritten value.
    const BEFORE: u32 = 123;
    const AFTER: u32 = 456;

    let m = Arc::new(Mutex::new(BEFORE));
    // Holder has the lock before the test thread calls the helper.
    let held = Arc::new(Barrier::new(2));
    // Test thread is about to call the helper: the holder keeps the
    // lock for a while past this point so the helper is deep in its
    // back-off, then releases.
    let helper_called = Arc::new(AtomicBool::new(false));

    let holder = {
        let m = Arc::clone(&m);
        let held = Arc::clone(&held);
        let helper_called = Arc::clone(&helper_called);
        thread::spawn(move || {
            let mut g = m.lock();
            held.wait();
            wait_until("the helper call", || helper_called.load(Ordering::Relaxed));
            // Past the helper's spin phase and several of its sleeps.
            thread::sleep(Duration::from_millis(5));
            *g = AFTER;
            let released_at = Instant::now();
            drop(g);
            released_at
        })
    };

    held.wait();
    helper_called.store(true, Ordering::Relaxed);
    let g = try_lock_with_backoff(&m);
    let acquired_at = Instant::now();
    let seen = *g;
    drop(g);
    let released_at = holder.join().expect("holder thread panicked");

    assert_eq!(
        seen, AFTER,
        "helper returned a guard before the holder released (saw {seen})",
    );
    assert!(
        acquired_at >= released_at,
        "helper acquired {:?} before the holder released",
        released_at - acquired_at
    );
    assert!(
        acquired_at - released_at < WEDGE,
        "helper took {:?} to wake after the release",
        acquired_at - released_at
    );
}
