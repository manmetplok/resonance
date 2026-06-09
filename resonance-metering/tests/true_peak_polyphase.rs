//! Equivalence tests for the linear-history polyphase rewrite.
//!
//! `PolyphasePeakDetector::push_sample` used to index its circular
//! history with a `%` per tap. The rewrite mirrors the history into a
//! double-length buffer so the inner loop is a linear walk. Both forms
//! visit the same samples in the same order with the same arithmetic, so
//! the held peak must match the old algorithm *bitwise* — this file pins
//! that with a faithful reimplementation of the old modulo-indexed loop.

use resonance_metering::true_peak::coefficients::{FIR, PHASES, TAPS};
use resonance_metering::true_peak::polyphase::PolyphasePeakDetector;

/// Reference: the pre-rewrite modulo-indexed detector, verbatim.
struct ReferenceDetector {
    history: [f32; TAPS],
    write_pos: usize,
    peak: f32,
}

impl ReferenceDetector {
    fn new() -> Self {
        Self {
            history: [0.0; TAPS],
            write_pos: 0,
            peak: 0.0,
        }
    }

    fn push_sample(&mut self, sample: f32) {
        self.history[self.write_pos] = sample;
        self.write_pos = (self.write_pos + 1) % TAPS;

        let abs_in = sample.abs();
        if abs_in > self.peak {
            self.peak = abs_in;
        }

        for taps in FIR.iter().take(PHASES) {
            let mut acc = 0.0_f32;
            for (j, &tap) in taps.iter().enumerate().take(TAPS) {
                let idx = (self.write_pos + TAPS - 1 - j) % TAPS;
                acc += tap * self.history[idx];
            }
            let abs = acc.abs();
            if abs > self.peak {
                self.peak = abs;
            }
        }
    }
}

/// Deterministic pseudo-random stream in [-1, 1] (xorshift).
fn noise(len: usize, mut state: u32) -> Vec<f32> {
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect()
}

#[test]
fn linear_history_matches_reference_bitwise_on_noise() {
    let input = noise(4096, 0x1234_5678);
    let mut new = PolyphasePeakDetector::new();
    let mut reference = ReferenceDetector::new();
    for (i, &s) in input.iter().enumerate() {
        new.push_sample(s);
        reference.push_sample(s);
        assert_eq!(
            new.peak(),
            reference.peak,
            "peak diverged at sample {i}"
        );
    }
}

#[test]
fn linear_history_matches_reference_on_intersample_peak_tone() {
    // fs/3 cosine with a phase offset — the classic inter-sample-peak
    // signal the oversampler exists for.
    let sr = 48_000.0_f32;
    let freq = 16_000.0_f32;
    let phase = std::f32::consts::PI / 6.0;
    let input: Vec<f32> = (0..8192)
        .map(|i| (phase + std::f32::consts::TAU * freq * i as f32 / sr).cos())
        .collect();

    let mut new = PolyphasePeakDetector::new();
    let mut reference = ReferenceDetector::new();
    for &s in &input {
        new.push_sample(s);
        reference.push_sample(s);
    }
    assert_eq!(new.peak(), reference.peak);
    // Sanity: the tone's true peak (≈1.0) exceeds its discrete peak (0.866).
    assert!(new.peak() > 0.95, "true peak {} too low", new.peak());
}

#[test]
fn push_block_matches_per_sample_and_reset_clears_history() {
    let input = noise(1024, 0xdead_beef);
    let mut blockwise = PolyphasePeakDetector::new();
    let mut samplewise = PolyphasePeakDetector::new();
    blockwise.push_block(&input);
    for &s in &input {
        samplewise.push_sample(s);
    }
    assert_eq!(blockwise.peak(), samplewise.peak());

    // After reset the detector must behave exactly like a fresh one.
    blockwise.reset();
    let mut fresh = PolyphasePeakDetector::new();
    blockwise.push_block(&input[..256]);
    fresh.push_block(&input[..256]);
    assert_eq!(blockwise.peak(), fresh.peak());
}
