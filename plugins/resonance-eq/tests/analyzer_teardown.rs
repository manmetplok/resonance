//! Worker-thread lifecycle for the spectrum analyzer: `initialize()`
//! spawns one background FFT worker per tap (pre + post), re-initializing
//! replaces them, and dropping the plugin joins them — no thread may
//! outlive its owner.
//!
//! The test used to compare the whole process's `Threads:` count against a
//! baseline, which flaked under load (FU-M6d): a joined thread stays listed
//! in /proc until the kernel reaps it, a moment *after* `pthread_join`
//! returns, and anything else in the process moved the total too. It now
//! tracks the analyzer's own workers by thread id and name
//! (`resonance-metering-spectrum`, truncated by the kernel to 15 bytes) and
//! waits — with a deadline — for the listing to settle:
//!
//! * a worker names itself from inside the new thread, so a fresh one can
//!   be listed unnamed for a moment;
//! * a joined worker's tid disappears only once it is reaped.
//!
//! Neither wait weakens the check: a leaked worker never exits, so its tid
//! never disappears and the deadline fails the test, and the exact count
//! of live workers is still asserted at each step.

#![cfg(target_os = "linux")]

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use resonance_eq::ResonanceEq;
use resonance_plugin::ResonancePlugin;

/// `resonance-metering-spectrum` as the kernel stores it (`TASK_COMM_LEN`
/// is 16 including the NUL).
const WORKER_COMM: &str = "resonance-meter";
const DEADLINE: Duration = Duration::from_secs(10);

/// Thread ids of this process's spectrum workers.
fn workers() -> BTreeSet<u32> {
    std::fs::read_dir("/proc/self/task")
        .expect("list /proc/self/task")
        .filter_map(|e| {
            let e = e.ok()?;
            let tid: u32 = e.file_name().to_str()?.parse().ok()?;
            let comm = std::fs::read_to_string(e.path().join("comm")).ok()?;
            (comm.trim_end() == WORKER_COMM).then_some(tid)
        })
        .collect()
}

/// Wait until exactly `n` workers are listed and none of `gone` is still
/// listed, then return the live set. Panics with `what` at the deadline.
fn settle(n: usize, gone: &BTreeSet<u32>, what: &str) -> BTreeSet<u32> {
    let start = Instant::now();
    loop {
        let now = workers();
        if now.len() == n && now.is_disjoint(gone) {
            return now;
        }
        assert!(
            start.elapsed() < DEADLINE,
            "{what}: {} spectrum workers listed ({now:?}), want {n}; \
             still listed of the ones that must be gone: {:?}",
            now.len(),
            now.intersection(gone).collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn spectrum_workers_are_joined_on_reinitialize_and_drop() {
    let none = BTreeSet::new();
    assert!(workers().is_empty(), "no spectrum worker before the plugin exists");

    let mut plugin = ResonanceEq::new();
    assert!(workers().is_empty(), "construction must not spawn threads");

    assert!(plugin.initialize(48_000.0, 256));
    let first = settle(2, &none, "initialize spawns exactly one worker per tap (pre + post)");

    // Re-initialize (e.g. a sample-rate change): the old workers must be
    // joined when the new pair replaces them, not accumulate.
    assert!(plugin.initialize(44_100.0, 256));
    let second = settle(2, &first, "re-initialize must replace the workers, not leak them");

    drop(plugin);
    settle(0, &second, "dropping the plugin must join both workers");
}
