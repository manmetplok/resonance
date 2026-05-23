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
//! `parking_lot::Mutex<u32>` so we can simulate contention
//! deterministically without spinning up a CLAP plugin.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use resonance_audio::__test_support::try_lock_with_backoff;

#[test]
fn uncontended_lock_returns_immediately() {
    // Sanity: with no other holder, the helper must take the fast
    // `try_lock` path and return without measurable delay.
    let m = Mutex::new(0u32);
    let start = Instant::now();
    let g = try_lock_with_backoff(&m);
    let elapsed = start.elapsed();
    drop(g);
    assert!(
        elapsed < Duration::from_millis(1),
        "uncontended lock took {elapsed:?}, expected sub-ms",
    );
}

#[test]
fn yields_lock_to_concurrent_try_lock_holder() {
    // The behaviour the original bug regressed: while the bounce-side
    // helper waits, an audio-thread-style `try_lock` from another
    // thread MUST be able to grab the mutex.
    //
    // We stage this by having the test thread hold the mutex, then
    // spawn the "bounce" thread which calls `try_lock_with_backoff`.
    // While the bounce thread is backing off, a third "audio" thread
    // takes a normal `try_lock` between releases. With the old
    // blocking `mutex.lock()`, the audio thread's try_lock would queue
    // behind the bounce thread; with the new helper the audio thread
    // hops in as soon as we release.

    let m = Arc::new(Mutex::new(0u32));
    let audio_got_lock = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));

    // Hold the lock so the bounce thread cannot get it on first try.
    let initial_guard = m.lock();

    // Bounce thread: hammers the helper until it eventually wins. We
    // only care that it makes progress eventually and doesn't starve
    // the audio thread in the meantime.
    let bounce_m = Arc::clone(&m);
    let bounce = thread::spawn(move || {
        let g = try_lock_with_backoff(&bounce_m);
        // Touch the value so the optimiser can't elide the guard.
        *g
    });

    // Audio thread: while the test thread holds the lock, the audio
    // thread's try_lock must fail (correct — there's a real holder).
    // The instant we drop the test thread's guard the audio thread
    // must be able to win at least once before the bounce thread
    // monopolises the mutex.
    let audio_m = Arc::clone(&m);
    let audio_flag = Arc::clone(&audio_got_lock);
    let audio_stop = Arc::clone(&stop);
    let audio = thread::spawn(move || {
        while !audio_stop.load(Ordering::Relaxed) {
            if let Some(_g) = audio_m.try_lock() {
                audio_flag.store(true, Ordering::Relaxed);
                // Tiny hold so the bounce thread also gets a chance.
                thread::sleep(Duration::from_micros(50));
            }
            thread::sleep(Duration::from_micros(20));
        }
    });

    // Give both worker threads time to start and reach their loops.
    thread::sleep(Duration::from_millis(5));

    // Release: the bounce thread is in its back-off sleep, so the
    // audio thread (which spins on `try_lock`) should grab the mutex
    // first.
    drop(initial_guard);

    // Wait until the bounce thread eventually wins (the helper retries
    // forever, so it will).
    let _value = bounce.join().expect("bounce thread panicked");
    stop.store(true, Ordering::Relaxed);
    audio.join().expect("audio thread panicked");

    assert!(
        audio_got_lock.load(Ordering::Relaxed),
        "audio thread never acquired the mutex — bounce helper is starving it",
    );
}

#[test]
fn bounce_helper_does_not_block_long_audio_holder() {
    // Stronger version of the previous test: while another thread
    // holds the mutex for a "long" time (5 ms — comfortably longer
    // than the helper's first few back-off intervals), the helper
    // must not panic, deadlock, or wake up before the holder releases.
    let m = Arc::new(Mutex::new(123u32));

    let holder_m = Arc::clone(&m);
    let holder = thread::spawn(move || {
        let _g = holder_m.lock();
        thread::sleep(Duration::from_millis(5));
        // Lock drops here.
    });

    // Give the holder time to grab the lock.
    thread::sleep(Duration::from_millis(1));

    let start = Instant::now();
    let g = try_lock_with_backoff(&m);
    let elapsed = start.elapsed();
    assert_eq!(*g, 123, "guard yielded the wrong value");
    drop(g);
    holder.join().expect("holder thread panicked");

    // Helper had to wait for the 5ms holder. Allow generous slack for
    // CI jitter, but at least confirm the helper slept rather than
    // returning a phantom guard.
    assert!(
        elapsed >= Duration::from_millis(3),
        "helper returned before the 5ms holder released (elapsed = {elapsed:?})",
    );
    assert!(
        elapsed < Duration::from_millis(50),
        "helper waited far longer than the holder ({elapsed:?})",
    );
}
