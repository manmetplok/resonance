//! Off-audio-thread FIR design for the linear-phase EQ and the multiband
//! crossovers (FU-M2a, FU-M2b / DSP-12).
//!
//! The worker must be invisible in the output: a design lands on the hop
//! boundary after the request whether the worker delivered it or the
//! inline fallback made it, so worker and inline-only renders are
//! bit-identical (live == bounce). The crossover also gets the EQ's
//! crossfade and rate-scaled FIR length.

use std::time::Duration;

use resonance_mastering::stages::linear_phase_eq::{
    BandConfig, BandType, DesignWorker, LinearPhaseEq, NUM_BANDS,
};
use resonance_mastering::stages::multiband::lowpass::LinearPhaseLowpass;
use resonance_mastering::stages::multiband::{Multiband, MultibandConfig};

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
    assert!(
        from_worker + inline_designs > 5,
        "automation never redesigned"
    );
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
    assert!(
        from_worker >= 3,
        "worker designs {from_worker}, inline {inline}"
    );
}

fn crossover_cfg(block: usize) -> MultibandConfig {
    let mut cfg = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };
    cfg.crossover_hz[0] = 120.0 + 80.0 * ((block as f32) * 0.07).sin().abs();
    cfg.crossover_hz[1] = 800.0 + 300.0 * ((block as f32) * 0.05).sin();
    for b in cfg.bands.iter_mut() {
        b.enabled = true;
        b.threshold_db = -30.0;
        b.ratio = 3.0;
    }
    cfg
}

fn render_multiband(mut mb: Multiband, input: &[f32]) -> Vec<f32> {
    let (mut l, mut r) = (input.to_vec(), input.to_vec());
    let mut pos = 0;
    let mut block = 0;
    while pos < l.len() {
        let n = 128.min(l.len() - pos);
        mb.process_stereo(
            &mut l[pos..pos + n],
            &mut r[pos..pos + n],
            &crossover_cfg(block),
        );
        pos += n;
        block += 1;
    }
    l
}

#[test]
fn crossover_output_is_identical_with_and_without_the_worker() {
    let input: Vec<f32> = tone(40_000, 150.0)
        .iter()
        .zip(tone(40_000, 900.0))
        .map(|(a, b)| a + b)
        .collect();
    let worker = DesignWorker::spawn();
    let with_worker = render_multiband(Multiband::with_worker(SR, 128, Some(&worker)), &input);
    let inline = render_multiband(Multiband::with_worker(SR, 128, None), &input);
    assert!(
        with_worker.iter().any(|v| v.abs() > 0.1),
        "multiband output is silent"
    );
    assert!(
        with_worker == inline,
        "worker-designed crossover output differs"
    );
}

/// DSP-12: a crossover move crossfades instead of swapping the FIR hard.
/// A sustained 800 Hz tone through a lowpass whose cutoff drops from
/// 2 kHz to 500 Hz changes level by ~15 dB; a hard swap steps the output
/// by `(h_new − h_old) * x` inside one sample.
#[test]
fn crossover_move_is_continuous() {
    let mut lp = LinearPhaseLowpass::with_worker(SR, 2000.0, None);
    let latency = lp.latency();
    let n = 3 * latency + 20_000;
    let (mut l, mut r) = (tone(n, 800.0), tone(n, 800.0));
    let switch = 2 * latency;
    lp.process_stereo(&mut l[..switch], &mut r[..switch]);
    lp.set_cutoff(500.0);
    lp.process_stereo(&mut l[switch..], &mut r[switch..]);

    let max_delta = |s: &[f32]| {
        s.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max)
    };
    let steady = max_delta(&l[latency + 1000..switch]);
    let after = max_delta(&l[switch..]);
    assert!(steady > 1e-3, "steady-state output is silent");
    assert!(
        after < 2.0 * steady,
        "crossover move stepped the output: max delta {after} vs steady {steady}"
    );
    // And the move did take effect.
    let tail_peak = l[n - 2000..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(
        tail_peak < 0.4 * 0.5,
        "cutoff move never landed: tail peak {tail_peak}"
    );
}

/// DSP-12 / FU-M2b: the crossover FIR scales with the sample rate like
/// the EQ's (DSP-06), keeping its low-band resolution and its latency in
/// ms.
#[test]
fn crossover_fir_scales_with_sample_rate() {
    assert_eq!(
        LinearPhaseLowpass::latency_for(96_000.0),
        2 * LinearPhaseLowpass::latency_for(48_000.0)
    );
    assert_eq!(
        Multiband::latency_for(96_000.0),
        Multiband::new(96_000.0, 64).latency()
    );

    // A low crossover still separates at 96 kHz: 400 Hz is two octaves
    // over a 100 Hz LR4 cutoff (~ −24 dB ideal).
    let sr = 96_000.0;
    let mut lp = LinearPhaseLowpass::with_worker(sr, 100.0, None);
    let n = lp.latency() + 40_000;
    let mk = || -> Vec<f32> {
        (0..n)
            .map(|i| 0.5 * (std::f32::consts::TAU * 400.0 * i as f32 / sr).sin())
            .collect()
    };
    let (mut l, mut r) = (mk(), mk());
    lp.process_stereo(&mut l, &mut r);
    let tail = &l[n - 20_000..];
    let rms = (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt();
    let db = 20.0 * (rms / (0.5 / 2f32.sqrt())).log10();
    assert!(
        db < -18.0,
        "400 Hz through a 100 Hz crossover at 96 kHz: {db:.1} dB"
    );
}
