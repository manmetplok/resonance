//! DSP2-04: the detector must measure the level whatever the stereo
//! image. It used to detect on the mono sum `0.5 * (l + r)`, which reads a
//! hard-panned source 6 dB low and an anti-phase one as silence, so both
//! were under-compressed. It now tracks the louder channel, after a
//! per-channel sidechain HPF, for the self-key and the external key alike.

use resonance_compressor::dsp::CompressorDsp;
use resonance_compressor::params::CompressorParams;
use resonance_compressor::viz::CompressorViz;
use resonance_plugin::Param;

const SR: f32 = 48_000.0;
const FRAMES: usize = 48_000;

fn params(sc_hpf: bool) -> CompressorParams {
    let p = CompressorParams::default();
    p.threshold.set_value(-20.0);
    p.ratio.set_value(4.0);
    p.attack.set_value(5.0);
    p.release.set_value(50.0);
    p.knee.set_value(0.0);
    p.makeup.set_value(0.0);
    p.auto_makeup.set_plain(0.0);
    p.mix.set_value(1.0);
    p.detector_mix.set_value(0.0);
    p.sc_hpf_on.set_plain(if sc_hpf { 1.0 } else { 0.0 });
    p.sc_hpf_freq.set_value(80.0);
    p
}

/// -6 dBFS, 1 kHz.
fn sine() -> Vec<f32> {
    let amp = 10f32.powf(-6.0 / 20.0);
    (0..FRAMES)
        .map(|i| amp * (2.0 * std::f32::consts::PI * 1_000.0 * i as f32 / SR).sin())
        .collect()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

#[derive(Clone, Copy)]
enum Image {
    Centred,
    LeftOnly,
    AntiPhase,
}

fn stereo(image: Image) -> (Vec<f32>, Vec<f32>) {
    let s = sine();
    match image {
        Image::Centred => (s.clone(), s),
        Image::LeftOnly => (s.clone(), vec![0.0; FRAMES]),
        Image::AntiPhase => (s.clone(), s.iter().map(|x| -x).collect()),
    }
}

/// Steady-state gain reduction, dB, on the left channel (which always
/// carries the full-level sine).
fn gain_reduction_db(image: Image, keyed: bool, sc_hpf: bool) -> f32 {
    let p = params(sc_hpf);
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &p);
    let (in_l, _) = stereo(image);
    let tail = FRAMES / 2..;
    let in_peak = peak(&in_l[tail.clone()]);
    let mut out = Vec::with_capacity(FRAMES);
    for chunk in 0..FRAMES / 512 {
        let range = chunk * 512..(chunk + 1) * 512;
        if keyed {
            // A quiet centred input, keyed by the stereo signal under test.
            let mut l: Vec<f32> = sine()[range.clone()].iter().map(|x| x * 0.01).collect();
            let mut r = l.clone();
            let (kl, kr) = stereo(image);
            dsp.process_stereo(
                &mut l,
                &mut r,
                Some((&kl[range.clone()], &kr[range.clone()])),
                &p,
                &viz,
            );
            out.extend(l.iter().map(|x| x * 100.0));
        } else {
            let (sl, sr) = stereo(image);
            let mut l = sl[range.clone()].to_vec();
            let mut r = sr[range.clone()].to_vec();
            dsp.process_stereo(&mut l, &mut r, None, &p, &viz);
            out.extend(l);
        }
    }
    20.0 * (in_peak / peak(&out[tail])).log10()
}

#[test]
fn panned_and_anti_phase_material_compress_like_centred() {
    for &sc_hpf in &[false, true] {
        for &keyed in &[false, true] {
            let centred = gain_reduction_db(Image::Centred, keyed, sc_hpf);
            // -6 dBFS peak against a -20 threshold at 4:1 is ~10 dB of GR.
            assert!(
                centred > 8.0,
                "centred GR {centred:.2} dB (keyed {keyed}, hpf {sc_hpf})"
            );
            for image in [Image::LeftOnly, Image::AntiPhase] {
                let gr = gain_reduction_db(image, keyed, sc_hpf);
                assert!(
                    (gr - centred).abs() < 0.5,
                    "GR {gr:.2} dB vs centred {centred:.2} dB (keyed {keyed}, hpf {sc_hpf})"
                );
            }
        }
    }
}
