//! DSP2-09: tuning a pad up must not alias.
//!
//! An upward-tuned voice reads `rate` take frames per output frame, so
//! take content above `fs / (2·rate)` lands past Nyquist and folds back
//! into the band. The 4-point Hermite read did nothing about it: hats and
//! cymbals tuned up got inharmonic grit.
//!
//! At +12 st (rate 2) a take whose content all sits at 13–23 kHz has
//! nothing that belongs in the output: every bit of it is folded energy,
//! and it must come out ≥ 40 dB below a take with the same content at
//! 1–8 kHz, which plays as it should (an octave up, at 2–16 kHz).

use resonance_drums::drum_map::{self, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, PortBuffers};
use resonance_drums::kit::{LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer, NUM_OUTPUT_PORTS};
use resonance_drums::params::DrumParams;
use resonance_plugin::Param;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

/// 40 equal-amplitude sines at pseudo-random frequencies in `lo..hi` Hz
/// with pseudo-random phases.
fn cluster(lo: f32, hi: f32, frames: usize) -> Vec<f32> {
    let mut s = 0x1234_5678u32;
    let mut unit = move || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        (s >> 8) as f64 / (1u32 << 24) as f64
    };
    let partials: Vec<(f64, f64)> = (0..40)
        .map(|_| (lo as f64 + (hi - lo) as f64 * unit(), std::f64::consts::TAU * unit()))
        .collect();
    (0..frames)
        .map(|i| {
            let t = i as f64 / SR as f64;
            let v: f64 = partials
                .iter()
                .map(|(f, p)| (std::f64::consts::TAU * f * t + p).sin())
                .sum();
            (v * 0.02) as f32
        })
        .collect()
}

fn kick_kit(take: LoadedSample) -> Vec<LoadedPad> {
    PAD_MAPPINGS
        .iter()
        .enumerate()
        .map(|(i, m)| LoadedPad {
            name: m.name.to_string(),
            choke_group: m.choke_group,
            output_group: m.output_group,
            close_mics: if i == 0 {
                vec![LoadedMicBank {
                    position: "KickIn".to_string(),
                    setup_key: String::new(),
                    layers: vec![VelocityLayer::new(vec![take.clone()])],
                }]
            } else {
                Vec::new()
            },
            extra_banks: Vec::new(),
            overhead: None,
        })
        .collect()
}

fn render_tuned(take: Vec<f32>, st: f32, frames: usize) -> Vec<f32> {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut s = DrumSampler::new(rx);
    s.set_sample_rate(SR);
    s.pads = kick_kit(LoadedSample::mono(take));
    let params = DrumParams::default();
    params.pads[0].tune.set_value(st);
    s.update_global_settings(&params);
    s.note_on(drum_map::KICK, 1.0);
    let mut out = Vec::with_capacity(frames);
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0; BLOCK], vec![0.0; BLOCK]))
        .collect();
    while out.len() < frames {
        {
            let mut ports: Vec<PortBuffers<'_>> = bufs
                .iter_mut()
                .map(|(l, r)| PortBuffers {
                    left: l.as_mut_slice(),
                    right: r.as_mut_slice(),
                })
                .collect();
            s.render_block(&mut ports, BLOCK, &params, &[]);
        }
        out.extend_from_slice(&bufs[0].0);
    }
    out.truncate(frames);
    out
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&v| v as f64 * v as f64).sum::<f64>() / x.len() as f64).sqrt()
}

#[test]
fn a_pad_tuned_up_an_octave_does_not_alias() {
    for st in [12.0, 24.0] {
        let rate = 2f32.powf(st / 12.0);
        let nyq = SR / (2.0 * rate);
        // Content the read must keep, and content that can only fold.
        let keep = render_tuned(cluster(500.0, 0.66 * nyq, 120_000), st, 20_000);
        let fold = render_tuned(cluster(1.1 * nyq, 23_000.0, 120_000), st, 20_000);
        let (keep, fold) = (rms(&keep[2_000..18_000]), rms(&fold[2_000..18_000]));
        assert!(keep > 1e-3, "{st:+} st: rendered silence");
        let db = 20.0 * (fold / keep).log10();
        assert!(db < -40.0, "{st:+} st: folded content only {db:.1} dB down");
    }
}
