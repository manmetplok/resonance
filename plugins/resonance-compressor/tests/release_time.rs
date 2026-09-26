//! LIB-06: the release knob is the release the user gets. After a loud
//! passage stops, gain reduction must fall to 1/e of its steady value in
//! one release time constant — not in the ~2x a second, cascaded release
//! stage on the peak detector would stretch it to.
//!
//! DC input (sidechain HPF off) makes `output / input` the exact per-sample
//! gain, so the GR trajectory is read straight off the audio.

use resonance_compressor::dsp::CompressorDsp;
use resonance_compressor::params::CompressorParams;
use resonance_compressor::viz::CompressorViz;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
const LOUD: f32 = 1.0; // 0 dBFS
const QUIET: f32 = 0.001; // -60 dBFS

fn params(release_ms: f32) -> CompressorParams {
    let p = CompressorParams::default();
    p.threshold.set_value(-20.0);
    p.ratio.set_value(4.0);
    p.knee.set_value(0.0);
    p.attack.set_value(1.0);
    p.release.set_value(release_ms);
    p.makeup.set_value(0.0);
    p.mix.set_value(1.0);
    p.detector_mix.set_value(0.0); // peak
    p.auto_makeup.set_value(false);
    p.sc_hpf_on.set_value(false);
    p
}

/// Samples from the loud→quiet step until GR falls to 1/e of its steady
/// value, plus the steady GR and the pre-step output level.
fn measure(release_ms: f32) -> (usize, f32, f32) {
    let params = params(release_ms);
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &params);

    let mut last_loud_out = 0.0;
    for _ in 0..(SR as usize / BLOCK) {
        let mut l = vec![LOUD; BLOCK];
        let mut r = vec![LOUD; BLOCK];
        dsp.process_stereo(&mut l, &mut r, None, &params, &viz);
        last_loud_out = l[BLOCK - 1];
    }
    let steady_gr = -20.0 * (last_loud_out / LOUD).log10();

    let target = steady_gr / std::f32::consts::E;
    let mut n = 0usize;
    for _ in 0..(4 * SR as usize / BLOCK) {
        let mut l = vec![QUIET; BLOCK];
        let mut r = vec![QUIET; BLOCK];
        dsp.process_stereo(&mut l, &mut r, None, &params, &viz);
        for &y in &l {
            let gr = -20.0 * (y / QUIET).log10();
            if gr <= target {
                return (n, steady_gr, last_loud_out);
            }
            n += 1;
        }
    }
    panic!("GR never released (steady {steady_gr} dB)");
}

#[test]
fn gain_reduction_releases_in_one_time_constant() {
    for release_ms in [50.0f32, 100.0, 300.0] {
        let (samples, steady_gr, loud_out) = measure(release_ms);
        assert!(loud_out > 0.05, "pre-step output must be non-silent ({loud_out})");
        assert!(
            (steady_gr - 15.0).abs() < 0.5,
            "0 dBFS at -20 dB / 4:1 must settle at 15 dB GR, got {steady_gr}"
        );
        let got_ms = samples as f32 / SR * 1000.0;
        assert!(
            (got_ms - release_ms).abs() <= 0.15 * release_ms,
            "release {release_ms} ms: GR took {got_ms:.1} ms to fall to 1/e"
        );
    }
}
