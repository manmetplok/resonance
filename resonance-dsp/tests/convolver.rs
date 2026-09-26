//! Tests for the shared streaming FFT convolver: identity/delay
//! behaviour, long-IR partitioning against direct convolution, chunked
//! streaming equivalence, reset, and in-place filter replacement.

use resonance_dsp::FftConvolver;

const HOP: usize = 128;

/// Deterministic xorshift noise in [-0.5, 0.5].
fn noise(len: usize, mut seed: u32) -> Vec<f32> {
    (0..len)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f32 / u32::MAX as f32) - 0.5
        })
        .collect()
}

/// Direct (time-domain) convolution reference.
fn direct_convolution(x: &[f32], h: &[f32]) -> Vec<f32> {
    let mut y = vec![0.0_f64; x.len()];
    for (n, out) in y.iter_mut().enumerate() {
        for (k, &tap) in h.iter().enumerate() {
            if n >= k {
                *out += f64::from(tap) * f64::from(x[n - k]);
            }
        }
    }
    y.into_iter().map(|v| v as f32).collect()
}

#[test]
fn unit_impulse_ir_is_identity_delayed_by_hop() {
    let mut c = FftConvolver::new(&[1.0], HOP);
    assert_eq!(c.latency(), HOP);
    assert_eq!(c.hop(), HOP);

    let input = noise(4 * HOP + 37, 0xABCD);
    let mut buf = input.clone();
    c.process_in_place(&mut buf);

    for i in 0..HOP {
        assert_eq!(buf[i], 0.0, "pre-latency sample {i} must be zero");
    }
    for i in HOP..buf.len() {
        let err = (buf[i] - input[i - HOP]).abs();
        assert!(err < 1e-5, "sample {i}: error {err} vs delayed input");
    }
}

#[test]
fn delayed_impulse_ir_delays_by_hop_plus_tap() {
    let tap = 50;
    let mut ir = vec![0.0_f32; tap + 1];
    ir[tap] = 1.0;
    let mut c = FftConvolver::new(&ir, HOP);

    let input = noise(5 * HOP, 0x1234);
    let mut buf = input.clone();
    c.process_in_place(&mut buf);

    let delay = HOP + tap;
    for i in 0..delay {
        assert!(
            buf[i].abs() < 1e-6,
            "pre-delay sample {i} = {} must be ~zero",
            buf[i]
        );
    }
    for i in delay..buf.len() {
        let err = (buf[i] - input[i - delay]).abs();
        assert!(err < 1e-5, "sample {i}: error {err} vs delayed input");
    }
}

#[test]
fn long_ir_partitioned_convolution_matches_direct_convolution() {
    // 5 hops + change → 6 partitions; exercises the FDL accumulation.
    let ir = noise(5 * HOP + 33, 0xBEEF);
    let mut c = FftConvolver::new(&ir, HOP);

    let input = noise(8 * HOP + 17, 0xF00D);
    let mut buf = input.clone();
    c.process_in_place(&mut buf);

    let expected = direct_convolution(&input, &ir);
    for i in HOP..buf.len() {
        let err = (buf[i] - expected[i - HOP]).abs();
        assert!(
            err < 1e-3,
            "sample {i}: streamed {} vs direct {} (err {err})",
            buf[i],
            expected[i - HOP]
        );
    }
}

#[test]
fn single_partition_ir_may_hold_hop_plus_one_taps() {
    // The overlap-save bound allows hop + 1 taps in one partition; the
    // mastering FIR (4097 taps at hop 4096) relies on this.
    let ir = noise(HOP + 1, 0x5EED);
    let mut c = FftConvolver::new(&ir, HOP);

    let input = noise(6 * HOP, 0xACE);
    let mut buf = input.clone();
    c.process_in_place(&mut buf);

    let expected = direct_convolution(&input, &ir);
    for i in HOP..buf.len() {
        let err = (buf[i] - expected[i - HOP]).abs();
        assert!(err < 1e-4, "sample {i}: err {err}");
    }
}

#[test]
fn chunked_streaming_is_bitwise_identical_to_one_shot() {
    let ir = noise(3 * HOP + 5, 0x77);
    let input = noise(7 * HOP + 191, 0x99);

    let mut whole = FftConvolver::new(&ir, HOP);
    let mut a = input.clone();
    whole.process_in_place(&mut a);

    let mut chunked = FftConvolver::new(&ir, HOP);
    let mut b = input;
    let mut offset = 0;
    for chunk in [1usize, 17, 128, 64, 300, 5, 129].iter().cycle() {
        if offset >= b.len() {
            break;
        }
        let end = (offset + chunk).min(b.len());
        chunked.process_in_place(&mut b[offset..end]);
        offset = end;
    }

    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert!(
            x.to_bits() == y.to_bits(),
            "sample {i} differs: one-shot={x:?} chunked={y:?}"
        );
    }
}

#[test]
fn per_sample_api_matches_block_api_bitwise() {
    let ir = noise(2 * HOP, 0x42);
    let input = noise(4 * HOP + 3, 0x43);

    let mut block = FftConvolver::new(&ir, HOP);
    let mut a = input.clone();
    block.process_in_place(&mut a);

    let mut per_sample = FftConvolver::new(&ir, HOP);
    let b: Vec<f32> = input.iter().map(|&x| per_sample.process_sample(x)).collect();

    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert!(x.to_bits() == y.to_bits(), "sample {i} differs");
    }
}

#[test]
fn reset_restores_initial_streaming_state_bitwise() {
    let ir = noise(3 * HOP, 0x1111);
    let mut fresh = FftConvolver::new(&ir, HOP);
    let mut reused = FftConvolver::new(&ir, HOP);

    // Dirty the reused instance's history, FDL, and FIFOs, then reset.
    let mut scratch = noise(5 * HOP + 77, 0x2222);
    reused.process_in_place(&mut scratch);
    reused.reset();

    let input = noise(4 * HOP, 0x3333);
    let mut a = input.clone();
    let mut b = input;
    fresh.process_in_place(&mut a);
    reused.process_in_place(&mut b);
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert!(
            x.to_bits() == y.to_bits(),
            "sample {i} differs after reset: fresh={x:?} reused={y:?}"
        );
    }
}

#[test]
fn set_impulse_response_swaps_filter_in_place() {
    let ir_a = noise(HOP, 0xAAAA);
    let ir_b = noise(HOP + 1, 0xBBBB);

    // Swap a→b, then reset: must match a convolver built with b.
    let mut swapped = FftConvolver::new(&ir_a, HOP);
    let mut scratch = noise(3 * HOP, 0xCCCC);
    swapped.process_in_place(&mut scratch);
    swapped.set_impulse_response(&ir_b);
    swapped.reset();

    let mut built = FftConvolver::new(&ir_b, HOP);

    let input = noise(4 * HOP, 0xDDDD);
    let mut a = input.clone();
    let mut b = input;
    swapped.process_in_place(&mut a);
    built.process_in_place(&mut b);
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert!(x.to_bits() == y.to_bits(), "sample {i} differs after swap");
    }
}

#[test]
fn empty_ir_yields_silence() {
    let mut c = FftConvolver::new(&[], HOP);
    let mut buf = noise(3 * HOP, 0xE0E0);
    c.process_in_place(&mut buf);
    assert!(buf.iter().all(|v| *v == 0.0), "empty IR must output silence");
}

/// DSP-10: a crossfaded IR change ramps the output linearly from the
/// old filter to the new one across exactly one hop, then holds the new
/// filter; it never steps.
#[test]
fn crossfaded_ir_change_ramps_over_one_hop() {
    let mut unity = vec![0.0_f32; HOP + 1];
    unity[0] = 1.0;
    let mut double = vec![0.0_f32; HOP + 1];
    double[0] = 2.0;
    let mut c = FftConvolver::new(&unity, HOP);
    // Prime with DC so the output is a steady 1.0.
    let mut out = Vec::new();
    for _ in 0..4 * HOP {
        out.push(c.process_sample(1.0));
    }
    c.crossfade_to_impulse_response(&double);
    assert!(c.crossfade_pending());
    for _ in 0..4 * HOP {
        out.push(c.process_sample(1.0));
    }
    assert!(!c.crossfade_pending(), "crossfade never landed");
    let tail = &out[2 * HOP..];
    assert!((tail[0] - 1.0).abs() < 1e-4, "not primed: {}", tail[0]);
    assert!((tail.last().unwrap() - 2.0).abs() < 1e-4, "new IR not adopted");
    let max_step = tail.windows(2).fold(0.0_f32, |m, w| m.max((w[1] - w[0]).abs()));
    assert!(
        max_step <= 1.0 / HOP as f32 + 1e-4,
        "crossfade stepped by {max_step} (one hop ramp is {})",
        1.0 / HOP as f32
    );
}

/// Before anything has been filtered, a crossfaded change applies at
/// once (there is nothing to fade from).
#[test]
fn crossfade_before_first_iteration_is_instant() {
    let mut unity = vec![0.0_f32; HOP + 1];
    unity[0] = 1.0;
    let mut half = vec![0.0_f32; HOP + 1];
    half[0] = 0.5;
    let mut c = FftConvolver::new(&unity, HOP);
    c.crossfade_to_impulse_response(&half);
    assert!(!c.crossfade_pending());
    let out: Vec<f32> = (0..3 * HOP).map(|_| c.process_sample(1.0)).collect();
    assert!((out[2 * HOP] - 0.5).abs() < 1e-4, "got {}", out[2 * HOP]);
}
