use resonance_dsp::SimpleRng;

/// Seed 0 must not degenerate: the constructor's `| 1` keeps the state
/// off xorshift's all-zero fixed point, so the stream is non-zero and
/// non-constant.
#[test]
fn zero_seed_produces_live_stream() {
    let mut rng = SimpleRng::new(0);
    let first = rng.next_u32();
    assert_ne!(first, 0);
    let mut saw_different = false;
    for _ in 0..16 {
        let v = rng.next_u32();
        assert_ne!(v, 0, "xorshift emitted 0 — state collapsed");
        if v != first {
            saw_different = true;
        }
    }
    assert!(saw_different, "stream is constant");
}

/// A seed whose high and low halves are equal XORs to 0 before the `| 1`
/// rescue — the other route into the degenerate state.
#[test]
fn equal_half_seed_produces_live_stream() {
    let mut rng = SimpleRng::new(0xDEAD_BEEF_DEAD_BEEF);
    for _ in 0..16 {
        assert_ne!(rng.next_u32(), 0, "xorshift emitted 0 — state collapsed");
    }
}

/// Same seed, same stream: the generator is deterministic.
#[test]
fn deterministic_for_equal_seeds() {
    let mut a = SimpleRng::new(42);
    let mut b = SimpleRng::new(42);
    for _ in 0..32 {
        assert_eq!(a.next_u32(), b.next_u32());
    }
}
