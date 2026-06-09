use resonance_mastering::stages::linear_phase_eq::convolver::OverlapSaveConvolver;
use resonance_mastering::stages::linear_phase_eq::HOP_SIZE;

fn delta_signal(len: usize) -> Vec<f32> {
    let mut v = vec![0.0_f32; len];
    v[0] = 1.0;
    v
}

#[test]
fn delta_through_identity_filter_appears_at_reported_latency() {
    let mut c = OverlapSaveConvolver::new();
    let latency = c.latency();
    // Feed enough samples to flush the delta through.
    let n = latency + HOP_SIZE;
    let mut buf = delta_signal(n);
    c.process_in_place(&mut buf);

    // The delta must land at index `latency` with near-unit magnitude.
    assert!(
        (buf[latency] - 1.0).abs() < 1e-4,
        "expected delta at index {latency}, got {}",
        buf[latency]
    );
    // Surrounding samples must be near zero.
    let mut max_other = 0.0_f32;
    for (i, &v) in buf.iter().enumerate() {
        if i != latency {
            max_other = max_other.max(v.abs());
        }
    }
    assert!(max_other < 1e-4, "non-delta ringing = {max_other}");
}

#[test]
fn sine_through_identity_is_delayed_copy() {
    let mut c = OverlapSaveConvolver::new();
    let latency = c.latency();
    let n = latency + 2048;
    let mut buf = vec![0.0_f32; n];
    for (i, v) in buf.iter_mut().enumerate() {
        *v = (i as f32 * 0.1).sin() * 0.5;
    }
    let input = buf.clone();
    c.process_in_place(&mut buf);

    // After `latency`, the output equals input shifted by `latency`.
    let mut max_err = 0.0_f32;
    for i in latency..n {
        let expected = input[i - latency];
        let err = (buf[i] - expected).abs();
        if err > max_err {
            max_err = err;
        }
    }
    assert!(max_err < 1e-4, "identity filter error = {max_err}");
}

// ---------------------------------------------------------------------------
// Flat-ring equivalence: the convolver's streaming FIFOs were converted
// from `VecDeque` to a flat ring with two indices. The FIFO swap must
// not change a single bit of output, so the reference below reimplements
// the previous `VecDeque`-based convolver verbatim (same FFT plan, same
// accumulation order) and the test compares bitwise.

mod vecdeque_reference {
    use std::collections::VecDeque;
    use std::sync::Arc;

    use resonance_mastering::stages::linear_phase_eq::convolver::{
        FFT_SIZE, FIR_LENGTH, GROUP_DELAY, HOP_SIZE,
    };
    use rustfft::num_complex::Complex;
    use rustfft::{Fft, FftPlanner};

    /// Verbatim copy of the pre-flat-ring `OverlapSaveConvolver`.
    pub struct Reference {
        fft_forward: Arc<dyn Fft<f32> + Send + Sync>,
        fft_inverse: Arc<dyn Fft<f32> + Send + Sync>,
        filter_response: Vec<Complex<f32>>,
        scratch: Vec<Complex<f32>>,
        input_history: Vec<f32>,
        input_pending: VecDeque<f32>,
        output_pending: VecDeque<f32>,
    }

    impl Reference {
        pub fn new() -> Self {
            let mut planner = FftPlanner::<f32>::new();
            let fft_forward = planner.plan_fft_forward(FFT_SIZE);
            let fft_inverse = planner.plan_fft_inverse(FFT_SIZE);

            let mut impulse = vec![0.0_f32; FIR_LENGTH];
            impulse[GROUP_DELAY] = 1.0;

            let mut c = Self {
                fft_forward,
                fft_inverse,
                filter_response: vec![Complex::new(0.0, 0.0); FFT_SIZE],
                scratch: vec![Complex::new(0.0, 0.0); FFT_SIZE],
                input_history: vec![0.0; FIR_LENGTH - 1],
                input_pending: VecDeque::with_capacity(HOP_SIZE * 2),
                output_pending: VecDeque::with_capacity(HOP_SIZE * 2),
            };
            c.set_impulse_response(&impulse);
            for _ in 0..HOP_SIZE {
                c.output_pending.push_back(0.0);
            }
            c
        }

        pub fn set_impulse_response(&mut self, h: &[f32]) {
            assert!(h.len() <= FIR_LENGTH);
            for (i, v) in self.scratch.iter_mut().enumerate() {
                if i < h.len() {
                    *v = Complex::new(h[i], 0.0);
                } else {
                    *v = Complex::new(0.0, 0.0);
                }
            }
            self.fft_forward.process(&mut self.scratch);
            self.filter_response.copy_from_slice(&self.scratch);
        }

        pub fn process_in_place(&mut self, buffer: &mut [f32]) {
            for sample in buffer.iter_mut() {
                self.input_pending.push_back(*sample);
                if self.input_pending.len() >= HOP_SIZE {
                    self.run_iteration();
                }
                *sample = self.output_pending.pop_front().unwrap_or(0.0);
            }
        }

        fn run_iteration(&mut self) {
            for i in 0..(FIR_LENGTH - 1) {
                self.scratch[i] = Complex::new(self.input_history[i], 0.0);
            }
            for i in 0..HOP_SIZE {
                let s = self.input_pending.pop_front().unwrap();
                self.scratch[FIR_LENGTH - 1 + i] = Complex::new(s, 0.0);
            }
            for i in 0..(FIR_LENGTH - 1) {
                let src = FFT_SIZE - (FIR_LENGTH - 1) + i;
                self.input_history[i] = self.scratch[src].re;
            }
            self.fft_forward.process(&mut self.scratch);
            for i in 0..FFT_SIZE {
                self.scratch[i] *= self.filter_response[i];
            }
            self.fft_inverse.process(&mut self.scratch);
            let norm = 1.0 / FFT_SIZE as f32;
            for i in 0..HOP_SIZE {
                let y = self.scratch[FIR_LENGTH - 1 + i].re * norm;
                self.output_pending.push_back(y);
            }
        }
    }
}

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

#[test]
fn flat_ring_is_bitwise_identical_to_vecdeque_reference() {
    use resonance_mastering::stages::linear_phase_eq::convolver::FIR_LENGTH;

    // A non-trivial asymmetric-magnitude FIR so the equivalence isn't
    // tested on the identity delta only.
    let mut fir = vec![0.0_f32; FIR_LENGTH];
    for (i, tap) in fir.iter_mut().enumerate() {
        let x = i as f32 / FIR_LENGTH as f32;
        *tap = (x * 37.0).sin() * (1.0 - x) * 0.01;
    }

    let mut ours = OverlapSaveConvolver::new();
    let mut reference = vecdeque_reference::Reference::new();
    ours.set_impulse_response(&fir);
    reference.set_impulse_response(&fir);

    // Stream > 3 hops of noise through both in irregular chunk sizes so
    // FFT iterations land mid-chunk and at chunk edges.
    let input = noise(3 * HOP_SIZE + 1234, 0xC0FFEE);
    let mut a = input.clone();
    let mut b = input;
    let mut offset = 0;
    for chunk in [1usize, 17, 480, 64, 1024, 4096, 333].iter().cycle() {
        if offset >= a.len() {
            break;
        }
        let end = (offset + chunk).min(a.len());
        ours.process_in_place(&mut a[offset..end]);
        reference.process_in_place(&mut b[offset..end]);
        offset = end;
    }

    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert!(
            x.to_bits() == y.to_bits(),
            "sample {i} differs: ours={x:?} reference={y:?}"
        );
    }
}

#[test]
fn reset_restores_initial_streaming_state_bitwise() {
    let mut fresh = OverlapSaveConvolver::new();
    let mut reused = OverlapSaveConvolver::new();

    // Dirty the reused instance's rings + history, then reset.
    let mut scratch = noise(HOP_SIZE + 777, 0xBADF00D);
    reused.process_in_place(&mut scratch);
    reused.reset();

    let input = noise(2 * HOP_SIZE, 42);
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
