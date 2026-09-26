//! Off-audio-thread FIR design for the linear-phase EQ (FU-M2a).
//!
//! The worker must be invisible in the output: a design lands on the hop
//! boundary after the request whether the worker delivered it or the
//! inline fallback made it, so worker and inline-only renders are
//! bit-identical (live == bounce).

use std::time::Duration;

use resonance_mastering::stages::linear_phase_eq::{
    BandConfig, BandType, LinearPhaseEq, NUM_BANDS,
};

const SR: f32 = 48_000.0;

fn tone(n: usize, hz: f32) -> Vec<f32> {
    (0..n)
        .map(|i| 0.4 * (std::f32::consts::TAU * hz * i as f32 / SR).sin())
        .collect()
}

fn bell(gain_db: f32) -> [BandConfig; NUM_BANDS] {
    let mut bands = [BandConfig::off(); NUM_BANDS];
    bands[0] = BandConfig {
        enabled: true,
        band_type: BandType::Bell,
        freq_hz: 1000.0,
        q: 1.0,
        gain_db,
    };
    bands
}

/// Render an EQ with its bell gain automated every block, in uneven
/// block sizes.
fn render_eq(mut eq: LinearPhaseEq, input: &[f32]) -> (Vec<f32>, LinearPhaseEq) {
    let (mut l, mut r) = (input.to_vec(), input.to_vec());
    let mut pos = 0;
    let mut block = 0;
    while pos < l.len() {
        let n = [128, 97, 256, 64][block % 4].min(l.len() - pos);
        let gain = 6.0 * ((block as f32) * 0.05).sin();
        eq.process_stereo(&mut l[pos..pos + n], &mut r[pos..pos + n], &bell(gain));
        pos += n;
        block += 1;
    }
    (l, eq)
}

#[test]
fn eq_output_is_identical_with_and_without_the_worker() {
    let input = tone(48_000, 1000.0);
    let (with_worker, eq) = render_eq(LinearPhaseEq::new(SR), &input);
    let (inline, _) = render_eq(LinearPhaseEq::with_worker(SR, None), &input);
    let (from_worker, inline_designs) = eq.design_counts();
    assert!(from_worker + inline_designs > 5, "automation never redesigned");
    assert!(
        with_worker.iter().any(|v| v.abs() > 0.1),
        "EQ output is silent"
    );
    assert!(
        with_worker == inline,
        "worker-designed EQ output differs from the inline-designed one"
    );
}

#[test]
fn eq_uses_the_worker_when_it_has_time() {
    let mut eq = LinearPhaseEq::new(SR);
    let (mut l, mut r) = (tone(64, 1000.0), tone(64, 1000.0));
    for step in 0..4 {
        // Request (one short block), give the worker ample time, then
        // run past the hop boundary where the design lands.
        eq.process_stereo(&mut l, &mut r, &bell(step as f32 + 1.0));
        std::thread::sleep(Duration::from_millis(200));
        let mut rest = vec![0.0f32; 8192];
        let mut rest_r = rest.clone();
        eq.process_stereo(&mut rest, &mut rest_r, &bell(step as f32 + 1.0));
    }
    let (from_worker, inline) = eq.design_counts();
    assert_eq!(from_worker + inline, 4);
    assert!(from_worker >= 3, "worker designs {from_worker}, inline {inline}");
}
