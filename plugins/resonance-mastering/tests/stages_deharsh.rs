//! The de-harsh stage in the mastering chain (warmth-width-depth.md W12,
//! `docs/design/deharsh-resonance-suppressor.md`): its params, the
//! constant reported latency, bit-transparency while it is off, the
//! resonance cut through the whole chain, and the editor panel.

use resonance_dsp::deharsh::StftGeometry;
use resonance_dsp::{Biquad, SimpleRng, SuppressorConfig};
use resonance_mastering::params::{MasteringParams, W12_PARAM_COUNT, W9_PARAM_COUNT};
use resonance_mastering::ResonanceMastering;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};
use rustfft::{num_complex::Complex, FftPlanner};

const SR: f32 = 48_000.0;
const MAX_BLOCK: usize = 512;
/// The chain's latency before W12, at 48 kHz: two linear-phase EQs, the
/// multiband crossover and the limiter lookahead.
const PRE_W12_LATENCY: usize = 18_673;

fn frame() -> usize {
    StftGeometry::for_sample_rate(SR).latency()
}

fn plugin_with(setup: impl Fn(&MasteringParams)) -> ResonanceMastering {
    let mut plugin = ResonanceMastering::new();
    setup(plugin.params());
    plugin.initialize(SR, MAX_BLOCK as u32);
    plugin.reset();
    plugin
}

/// Stream `l`/`r` through the plugin in `block`-sized chunks, calling
/// `edit(params, block_index)` before each block.
fn render(
    plugin: &mut ResonanceMastering,
    l: &[f32],
    r: &[f32],
    block: usize,
    mut edit: impl FnMut(&MasteringParams, usize),
) -> (Vec<f32>, Vec<f32>) {
    let (mut ol, mut or) = (l.to_vec(), r.to_vec());
    let mut start = 0;
    let mut b = 0;
    while start < ol.len() {
        let end = (start + block).min(ol.len());
        edit(plugin.params(), b);
        let mut outs = [OutputBuffer {
            left: &mut ol[start..end],
            right: &mut or[start..end],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, end - start, &mut ev, None);
        start = end;
        b += 1;
    }
    (ol, or)
}

fn white(seed: u64, n: usize) -> Vec<f32> {
    let mut rng = SimpleRng::new(seed);
    (0..n)
        .map(|_| (rng.next_u32() as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32)
        .collect()
}

/// Paul Kellet's refined pink filter over seeded white noise, scaled to
/// `rms_db`.
fn pink(seed: u64, n: usize, rms_db: f32) -> Vec<f32> {
    let w = white(seed, n);
    let mut b = [0.0f64; 7];
    let mut x: Vec<f32> = w
        .iter()
        .map(|&v| {
            let v = v as f64;
            b[0] = 0.99886 * b[0] + v * 0.0555179;
            b[1] = 0.99332 * b[1] + v * 0.0750759;
            b[2] = 0.96900 * b[2] + v * 0.1538520;
            b[3] = 0.86650 * b[3] + v * 0.3104856;
            b[4] = 0.55000 * b[4] + v * 0.5329522;
            b[5] = -0.7616 * b[5] - v * 0.0168980;
            let y = b.iter().sum::<f64>() + v * 0.5362;
            b[6] = v * 0.115926;
            y as f32
        })
        .collect();
    let rms = (x.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
    let g = 10f64.powf(rms_db as f64 / 20.0) / rms;
    for v in &mut x {
        *v = (*v as f64 * g) as f32;
    }
    x
}

fn bell(x: &[f32], freq: f32, q: f32, gain_db: f32) -> Vec<f32> {
    let mut bq = Biquad::default();
    bq.set_bell(SR, freq, q, gain_db);
    x.iter().map(|&v| bq.process(v)).collect()
}

/// Welch-averaged energy (Hann 8192, hop 4096) between `f1` and `f2`, dB.
fn band_db(x: &[f32], f1: f32, f2: f32) -> f64 {
    let n = 8192;
    let fft = FftPlanner::new().plan_fft_forward(n);
    let (lo, hi) = (
        (f1 / SR * n as f32).ceil() as usize,
        (f2 / SR * n as f32).floor() as usize,
    );
    let mut e = 0.0f64;
    let mut buf = vec![Complex::new(0.0f64, 0.0); n];
    let mut start = 0;
    while start + n <= x.len() {
        for i in 0..n {
            let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
            buf[i] = Complex::new(x[start + i] as f64 * w, 0.0);
        }
        fft.process(&mut buf);
        e += buf[lo..=hi].iter().map(|c| c.norm_sqr()).sum::<f64>();
        start += n / 2;
    }
    10.0 * e.max(1e-300).log10()
}

fn dh_on(p: &MasteringParams, depth: f32) {
    p.deharsh.on.set_value(true);
    p.deharsh.depth.set_value(depth);
}

// --- Params. ----------------------------------------------------------

#[test]
fn deharsh_params_are_appended_after_the_w9_block() {
    let p = MasteringParams::default();
    assert_eq!(W12_PARAM_COUNT, W9_PARAM_COUNT + 11);
    let ids: Vec<&str> = (W9_PARAM_COUNT..W12_PARAM_COUNT).map(|i| p.param_at(i).id()).collect();
    assert_eq!(
        ids,
        [
            "dh_on",
            "dh_depth",
            "dh_selectivity",
            "dh_sharpness",
            "dh_attack",
            "dh_release",
            "dh_low",
            "dh_high",
            "dh_mode",
            "dh_mix",
            "dh_delta"
        ]
    );
    // Earlier indices keep their ids (host automation lanes bind to them).
    assert_eq!(p.param_at(W9_PARAM_COUNT - 1).id(), "sat_curve");
}

#[test]
fn deharsh_param_defaults_are_the_suppressor_defaults() {
    let p = MasteringParams::default();
    assert_eq!(p.deharsh.snapshot(), SuppressorConfig::default());
    assert!(!p.deharsh.on.value(), "de-harsh must default off");
}

// --- Latency. ---------------------------------------------------------

#[test]
fn latency_is_the_pre_w12_sum_plus_one_frame_in_every_state() {
    let mut plugin = plugin_with(|_| {});
    let expected = PRE_W12_LATENCY + frame();
    assert_eq!(frame(), 2048);
    assert_eq!(plugin.latency_samples() as usize, expected);
    let x = pink(1, 48_000, -18.0);
    // Every dh param moves while audio runs, including on/off and every
    // mode and delta.
    let _ = render(&mut plugin, &x, &x, 256, |p, b| {
        p.deharsh.on.set_value(b % 40 >= 10);
        p.deharsh.depth.set_value((b % 25) as f32);
        p.deharsh.selectivity.set_value((b % 19) as f32);
        p.deharsh.sharpness.set_value(3.0 + (b % 22) as f32);
        p.deharsh.mode.set_value((b / 7 % 4) as i32);
        p.deharsh.delta.set_value(b % 17 == 3);
        p.deharsh.mix.set_value((b % 5) as f32 / 4.0);
        assert_eq!(p.deharsh.on.value(), b % 40 >= 10);
    });
    assert_eq!(plugin.latency_samples() as usize, expected);
    for sr in [44_100.0, 96_000.0] {
        let mut a = ResonanceMastering::new();
        a.initialize(sr, MAX_BLOCK as u32);
        let mut b = ResonanceMastering::new();
        b.params().deharsh.on.set_value(true);
        b.initialize(sr, MAX_BLOCK as u32);
        assert_eq!(a.latency_samples(), b.latency_samples(), "at {sr} Hz");
    }
}

// --- Never engaged: bit-transparent. ----------------------------------

#[test]
fn an_off_stage_is_inert_whatever_its_other_params() {
    let x = pink(2, 60_000, -12.0);
    let y = pink(3, 60_000, -14.0);
    let setup = |p: &MasteringParams| {
        p.limiter.on.set_value(true);
        p.glue_compressor.on.set_value(true);
        p.tonal_eq.bands[2].on.set_value(true);
    };
    let mut a = plugin_with(setup);
    let (al, ar) = render(&mut a, &x, &y, 333, |_, _| {});
    let mut b = plugin_with(setup);
    let (bl, br) = render(&mut b, &x, &y, 333, |p, blk| {
        // Off, but everything else moving.
        p.deharsh.depth.set_value((blk % 24) as f32);
        p.deharsh.selectivity.set_value(0.0);
        p.deharsh.mode.set_value((blk % 4) as i32);
        p.deharsh.delta.set_value(blk % 2 == 0);
        p.deharsh.mix.set_value(0.5);
    });
    assert!(al.iter().any(|v| v.abs() > 1e-3), "silent render");
    for i in 0..al.len() {
        assert_eq!(al[i].to_bits(), bl[i].to_bits(), "L differs at {i}");
        assert_eq!(ar[i].to_bits(), br[i].to_bits(), "R differs at {i}");
    }
}

#[test]
fn all_stages_off_is_the_input_delayed_by_the_latency() {
    let mut plugin = plugin_with(|_| {});
    let lat = plugin.latency_samples() as usize;
    let x = pink(4, lat + 8_000, -12.0);
    let (ol, _) = render(&mut plugin, &x, &x, 480, |_, _| {});
    let mut err = 0.0f32;
    for i in lat..x.len() {
        err = err.max((ol[i] - x[i - lat]).abs());
    }
    // The flat linear-phase FIRs round; everything else is exact.
    assert!(err < 1e-5, "off chain error {err}");
}

// --- Engaged: the exit criterion through the whole chain. -------------

#[test]
fn a_3k2_resonance_is_cut_at_least_6_db_through_the_chain() {
    let n = 6 * 48_000;
    let x = bell(&pink(5, n, -18.0), 3200.0, 10.0, 15.0);
    let mut plugin = plugin_with(|p| dh_on(p, 12.0));
    let lat = plugin.latency_samples() as usize;
    let (ol, _) = render(&mut plugin, &x, &x, 512, |_, _| {});
    let skip = 24_000;
    let (i, o) = (&x[skip..n - lat], &ol[skip + lat..]);
    let (f1, f2) = (3200.0 * 2f32.powf(-1.0 / 24.0), 3200.0 * 2f32.powf(1.0 / 24.0));
    let cut = band_db(i, f1, f2) - band_db(o, f1, f2);
    assert!(cut >= 6.0, "chain cut the resonance only {cut:.2} dB");
    let broad = band_db(o, 1000.0, 2000.0) - band_db(i, 1000.0, 2000.0);
    assert!(broad.abs() < 0.5, "1–2 kHz moved {broad:.2} dB");
}

// --- Editor. ----------------------------------------------------------

/// The panel is one row: the On checkbox, a Mode/Delta column (one
/// combo width) and eight 64 px knobs. It must fit the window, and every
/// knob readout must carry its unit.
#[cfg(feature = "editor")]
#[test]
fn the_deharsh_panel_fits_and_its_readouts_carry_units() {
    use resonance_mastering::editor::controls::{stage_panel_height, StageTab};
    use resonance_mastering::editor::WINDOW_W;
    use resonance_plugin::Param;

    let row = 8.0 + 60.0 + 8.0 + 108.0 + 8.0 + 8.0 * 64.0;
    assert!(row < WINDOW_W as f32, "de-harsh row needs {row} px of {WINDOW_W}");
    assert_eq!(stage_panel_height(StageTab::Deharsh), 260.0);

    let d = MasteringParams::default().deharsh;
    assert_eq!(d.depth.display(6.0), "6.0 dB");
    assert_eq!(d.selectivity.display(5.0), "5.0 dB");
    assert_eq!(d.sharpness.display(24.0), "Q 24.0");
    assert_eq!(d.attack.display(10.0), "10.0 ms");
    assert_eq!(d.release.display(100.0), "100 ms");
    assert_eq!(d.mix.display(1.0), "100%");
    for v in [d.low.display(1000.0), d.high.display(8000.0)] {
        assert!(v.ends_with("Hz"), "band edge reads {v:?}");
    }
    assert_eq!(d.mode.display(3.0), "Mid+Side");
}

/// DSP2-11 (checked, no change needed): a mode switch resets the detector
/// that stops being used, so its cut starts over, but every frame's gains
/// are applied in the STFT domain and overlap-added under the synthesis
/// window, so a change of cut is spread over a frame instead of landing on
/// one sample. Switching modes under a deep cut of a centred resonance
/// (Stereo → Side drops the cut entirely, → Mid brings it back) must not
/// step more than the signal's own slope.
#[test]
fn a_mode_switch_under_a_deep_cut_does_not_click() {
    use resonance_dsp::{SuppressorMode, SuppressorConfig};
    use resonance_mastering::stages::deharsh::DeharshStage;
    let n = 120 * 256;
    let tone: Vec<f32> = (0..n)
        .map(|i| 0.5 * (i as f32 / SR * 3_200.0 * std::f32::consts::TAU).sin())
        .collect();
    let bed = pink(9, n, -30.0);
    let x: Vec<f32> = tone.iter().zip(&bed).map(|(a, b)| a + b).collect();
    let (mut l, mut r) = (x.clone(), x.clone());
    let mut stage = DeharshStage::new(SR);
    let seq = [
        (0, SuppressorMode::Stereo),
        (40, SuppressorMode::Side),
        (80, SuppressorMode::Mid),
    ];
    let cfg = |mode| SuppressorConfig {
        enabled: true,
        depth_db: 12.0,
        mode,
        ..SuppressorConfig::default()
    };
    let mut mode = SuppressorMode::Stereo;
    for (k, start) in (0..n).step_by(256).enumerate() {
        if let Some((_, m)) = seq.iter().find(|(at, _)| *at == k) {
            mode = *m;
        }
        stage.process_stereo(&mut l[start..start + 256], &mut r[start..start + 256], &cfg(mode));
    }
    // The same input through a stage that ran one mode throughout.
    let steady = |m| {
        let (mut sl, mut sr) = (x.clone(), x.clone());
        let mut st = DeharshStage::new(SR);
        for start in (0..n).step_by(256) {
            st.process_stereo(&mut sl[start..start + 256], &mut sr[start..start + 256], &cfg(m));
        }
        sl
    };
    let max_step = |x: &[f32]| x.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
    let tone_step = 0.5 * std::f32::consts::TAU * 3_200.0 / SR;
    let cut = max_step(&l[30 * 256..40 * 256]);
    assert!(cut < 0.6 * tone_step, "the resonance is not being cut ({cut} vs {tone_step})");
    for &(at, m) in &seq[1..] {
        let window = at * 256..(at + 20) * 256;
        let seam = max_step(&l[window.clone()]);
        // Whichever of the two modes' steady outputs moves more.
        let prev = seq.iter().rev().find(|(a, _)| *a < at).unwrap().1;
        let bound = max_step(&steady(m)[window.clone()]).max(max_step(&steady(prev)[window]));
        assert!(
            seam < 1.1 * bound,
            "switch to {m:?} stepped {seam} vs {bound} for a steady run"
        );
    }
}
