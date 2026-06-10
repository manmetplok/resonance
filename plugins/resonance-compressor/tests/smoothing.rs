//! Makeup gain and mix must be de-zippered: host automation lands on the
//! params instantly (per the `Param::set_plain` contract), and the DSP's
//! smoothers turn the step into a short ramp instead of a per-sample
//! discontinuity. A constant DC input far below the threshold (with the
//! knee closed) produces zero gain reduction, so the per-sample output
//! gain trajectory exposes the makeup/mix smoothers directly.

use resonance_compressor::dsp::CompressorDsp;
use resonance_compressor::params::CompressorParams;
use resonance_compressor::viz::CompressorViz;

const SR: f32 = 48_000.0;
const BLOCK: usize = 512;
const SMOOTH_MS: f32 = 20.0;
const DC: f32 = 0.01; // -40 dBFS, far below the test threshold

fn quiet_params() -> CompressorParams {
    let params = CompressorParams::default();
    params.threshold.set_value(0.0);
    params.knee.set_value(0.0);
    params.makeup.set_value(0.0);
    params.mix.set_value(1.0);
    params.auto_makeup.set_value(false);
    params.sc_hpf_on.set_value(false);
    params
}

fn process_blocks(
    dsp: &mut CompressorDsp,
    params: &CompressorParams,
    viz: &CompressorViz,
    n_blocks: usize,
    out: &mut Vec<f32>,
) {
    for _ in 0..n_blocks {
        let mut left = vec![DC; BLOCK];
        let mut right = vec![DC; BLOCK];
        dsp.process_stereo(&mut left, &mut right, params, viz);
        out.extend_from_slice(&left);
    }
}

#[test]
fn makeup_step_ramps_without_per_sample_discontinuity() {
    const PRE_BLOCKS: usize = 4;
    const POST_BLOCKS: usize = 20;
    const STEP_DB: f32 = 12.0;
    let jump_at = PRE_BLOCKS * BLOCK;

    let params = quiet_params();
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &params);

    let mut out = Vec::new();
    process_blocks(&mut dsp, &params, &viz, PRE_BLOCKS, &mut out);
    params.makeup.set_value(STEP_DB);
    process_blocks(&mut dsp, &params, &viz, POST_BLOCKS, &mut out);

    // The makeup smoother is Logarithmic(20 ms): the largest per-sample dB
    // movement is the terminal snap-to-target after N ramp samples, which
    // sits at the residual exponential distance STEP_DB * e^(-3(N-1)/N) —
    // about 0.6 dB here, tiny next to the raw 12 dB jump an unsmoothed
    // read would produce.
    let ramp_samples = (SR * SMOOTH_MS / 1000.0).ceil();
    let bound = STEP_DB * (-3.0_f32 * (ramp_samples - 1.0) / ramp_samples).exp() + 1e-3;

    let mut max_delta_db = 0.0_f32;
    for n in 1..out.len() {
        let delta_db = 20.0 * (out[n] / out[n - 1]).abs().log10();
        max_delta_db = max_delta_db.max(delta_db.abs());
        assert!(
            delta_db.abs() <= bound,
            "gain discontinuity at sample {n} (jump at {jump_at}): \
             {delta_db} dB in one sample (allowed {bound})"
        );
    }
    // Sanity: the step actually moved the gain (the loop above would also
    // pass on a stuck output).
    assert!(max_delta_db > 1e-4, "output gain never moved");

    // The ramp converges: the per-block retarget restarts the exponential
    // approach, so convergence is asymptotic — by the last block (~160 ms
    // past the jump) the residual is far below 0.01 dB.
    let settled = out.len() - BLOCK;
    for (n, &x) in out.iter().enumerate().skip(settled) {
        let gain_db = 20.0 * (x / DC).log10();
        assert!(
            (gain_db - STEP_DB).abs() < 0.01,
            "gain not settled at sample {n}: {gain_db} dB"
        );
    }
}

#[test]
fn mix_step_ramps_without_per_sample_discontinuity() {
    const PRE_BLOCKS: usize = 4;
    const POST_BLOCKS: usize = 20;
    let jump_at = PRE_BLOCKS * BLOCK;

    // With makeup at +12 dB the wet path differs from dry, so the mix
    // knob has an audible effect to smooth.
    let params = quiet_params();
    params.makeup.set_value(12.0);
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &params);

    let mut out = Vec::new();
    process_blocks(&mut dsp, &params, &viz, PRE_BLOCKS, &mut out);
    params.mix.set_value(0.0);
    process_blocks(&mut dsp, &params, &viz, POST_BLOCKS, &mut out);

    // Linear(20 ms) mix smoother: per-sample mix movement is 1/ramp_samples,
    // scaled by the wet/dry gain difference.
    let ramp_samples = (SR * SMOOTH_MS / 1000.0).ceil();
    let wet_gain = 10.0_f32.powf(12.0 / 20.0);
    let bound = DC * (wet_gain - 1.0) / ramp_samples + 1e-7;

    for n in 1..out.len() {
        let delta = (out[n] - out[n - 1]).abs();
        assert!(
            delta <= bound,
            "output discontinuity at sample {n} (jump at {jump_at}): \
             {delta} in one sample (allowed {bound})"
        );
    }

    // Fully dry after the ramp (asymptotically — see the makeup test).
    let settled = out.len() - BLOCK;
    for (n, &x) in out.iter().enumerate().skip(settled) {
        assert!(
            (x - DC).abs() < 1e-6,
            "mix not settled at sample {n}: {x}"
        );
    }
}
