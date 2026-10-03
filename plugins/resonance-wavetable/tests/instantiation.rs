//! Instantiation cost: a project with several wavetable tracks builds one
//! `SynthEngine` per plugin instance, and each used to parse the ~36 MB
//! wavetable bundle into its own heap tree.
//!
//! The tables are now borrowed from the embedded blob, so `initialize()` does
//! no bulk copying and every instance shares the same read-only pages. These
//! tests pin that property: they would fail loudly if someone reintroduced a
//! per-instance copy.

use std::time::Instant;

use resonance_wavetable::dsp::engine::SynthEngine;

const SR: f32 = 48_000.0;

/// Parsing the bundle into per-instance `Vec`s took tens of milliseconds and
/// ~36 MB of allocation. Borrowing it is sub-millisecond. The bound is
/// deliberately loose (a 40x margin over what this actually costs) so the test
/// is not flaky under load, while still catching a reintroduced bulk copy.
#[test]
fn initialize_is_cheap() {
    // Warm the page cache / first-touch faults.
    let mut warm = SynthEngine::new();
    warm.initialize(SR);

    let start = Instant::now();
    const N: usize = 8;
    let mut engines = Vec::with_capacity(N);
    for _ in 0..N {
        let mut e = SynthEngine::new();
        e.initialize(SR);
        engines.push(e);
    }
    let per_instance = start.elapsed() / N as u32;

    eprintln!("initialize() = {per_instance:?} per instance");
    assert!(
        per_instance.as_millis() < 20,
        "initialize() took {per_instance:?} per instance — did the wavetable \
         bundle go back to being copied per instance?"
    );
}

/// Every instance must hand out views over the *same* backing memory. If a
/// future change reintroduces per-instance table storage, the pointers will
/// diverge and this fails.
#[test]
fn instances_share_wavetable_storage() {
    let mut a = SynthEngine::new();
    a.initialize(SR);
    let mut b = SynthEngine::new();
    b.initialize(SR);

    assert!(!a.wavetables.is_empty());
    assert_eq!(a.wavetables.len(), b.wavetables.len());

    for (ta, tb) in a.wavetables.iter().zip(b.wavetables.iter()) {
        assert_eq!(ta.num_frames(), tb.num_frames());
        assert_eq!(
            ta.mip(0, 0).as_ptr(),
            tb.mip(0, 0).as_ptr(),
            "wavetable storage is not shared between instances"
        );
    }
}

/// DSP2-16: parameter ids and names built at runtime are interned, so a
/// second instance reuses the first one's strings instead of leaking its
/// own copy of every one.
#[test]
fn param_ids_and_names_are_shared_between_instances() {
    use resonance_wavetable::params::{WavetableParams, PARAM_COUNT};
    let a = WavetableParams::new();
    let b = WavetableParams::new();
    for i in 0..PARAM_COUNT {
        let (pa, pb) = (a.param_at(i), b.param_at(i));
        assert_eq!(pa.id().as_ptr(), pb.id().as_ptr(), "id {} leaked again", pa.id());
        assert_eq!(pa.name().as_ptr(), pb.name().as_ptr(), "name {} leaked again", pa.name());
    }
}
