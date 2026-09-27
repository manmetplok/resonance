//! Isolated cost of the NAM matrix-vector kernel.
//!
//! `matvec` was the innermost loop of the sample-serial WaveNet forward
//! pass. The block forward pass now runs the convs through the block GEMM
//! (`nam::gemm`, measured end-to-end by `amp_dsp`); `matvec` remains on
//! the per-frame FiLM and head-MLP paths and in the LSTM. The shapes below
//! are the ones a stock "standard" NAM capture issues.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use resonance_amp::nam::matvec;

fn bench_matvec(c: &mut Criterion) {
    let mut group = c.benchmark_group("matvec");

    // (rows, cols) shapes from the standard WaveNet architecture:
    //   32x16 — the combined filter+gate dilated conv (mid_ch x channels)
    //   16x16 — layer1x1 residual and head1x1 skip projections
    //    8x16 — head rechannel (head_size x channels)
    for (rows, cols) in [(32usize, 16usize), (16, 16), (8, 16), (16, 8)] {
        let a: Vec<f32> = (0..rows * cols)
            .map(|i| ((i % 17) as f32 - 8.0) * 0.03)
            .collect();
        let x: Vec<f32> = (0..cols).map(|i| ((i % 7) as f32 - 3.0) * 0.1).collect();
        let mut y = vec![0.0f32; rows];

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{rows}x{cols}")),
            &(rows, cols),
            |b, &(rows, cols)| {
                b.iter(|| {
                    matvec(
                        black_box(&a),
                        black_box(&x),
                        rows,
                        cols,
                        black_box(&mut y),
                    );
                    black_box(y[0])
                })
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_matvec);
criterion_main!(benches);
