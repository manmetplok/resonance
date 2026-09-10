//! Worker-thread lifecycle for the spectrum analyzer: `initialize()`
//! spawns one background FFT worker per tap (pre + post), re-initializing
//! replaces them, and dropping the plugin joins them — no thread may
//! outlive its owner. `SpectrumAnalyzer`'s `Drop` joins synchronously,
//! so the counts below are deterministic.
//!
//! This file deliberately contains a SINGLE test: it counts the whole
//! process's threads via /proc, and a concurrent test spawning its own
//! plugin would skew the numbers.

#![cfg(target_os = "linux")]

use resonance_eq::ResonanceEq;
use resonance_plugin::ResonancePlugin;

/// Current number of threads in this process, from /proc/self/status.
fn thread_count() -> usize {
    let status = std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    status
        .lines()
        .find_map(|l| l.strip_prefix("Threads:"))
        .expect("Threads: line present")
        .trim()
        .parse()
        .expect("thread count parses")
}

#[test]
fn spectrum_workers_are_joined_on_reinitialize_and_drop() {
    let baseline = thread_count();

    let mut plugin = ResonanceEq::new();
    assert_eq!(
        thread_count(),
        baseline,
        "construction must not spawn threads"
    );

    assert!(plugin.initialize(48_000.0, 256));
    assert_eq!(
        thread_count(),
        baseline + 2,
        "initialize spawns exactly one worker per tap (pre + post)"
    );

    // Re-initialize (e.g. a sample-rate change): the old workers must be
    // joined when the new pair replaces them, not accumulate.
    assert!(plugin.initialize(44_100.0, 256));
    assert_eq!(
        thread_count(),
        baseline + 2,
        "re-initialize must replace the workers, not leak them"
    );

    drop(plugin);
    assert_eq!(
        thread_count(),
        baseline,
        "dropping the plugin must join both workers"
    );
}
