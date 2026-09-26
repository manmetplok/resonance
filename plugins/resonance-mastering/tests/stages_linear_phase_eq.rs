use resonance_mastering::stages::linear_phase_eq::{
    BandConfig, BandType, LinearPhaseEq, NUM_BANDS,
};

#[test]
fn default_eq_is_pure_delay() {
    let mut eq = LinearPhaseEq::new(48_000.0);
    let latency = eq.latency();
    let n = latency + 2048;
    let mut left = vec![0.0_f32; n];
    let mut right = vec![0.0_f32; n];
    for i in 0..n {
        let s = (i as f32 * 0.05).sin() * 0.3;
        left[i] = s;
        right[i] = s;
    }
    let input = left.clone();
    eq.process_stereo(&mut left, &mut right, &[BandConfig::off(); NUM_BANDS]);
    // Output at latency offset matches input with small tolerance.
    let mut max_err = 0.0_f32;
    for i in latency..n {
        max_err = max_err.max((left[i] - input[i - latency]).abs());
        max_err = max_err.max((right[i] - input[i - latency]).abs());
    }
    assert!(max_err < 5e-3, "default EQ error = {max_err}");
}

#[test]
fn bell_cut_attenuates_centre_frequency() {
    // -12 dB bell at 1 kHz: a 1 kHz sine should come out ~4× quieter
    // after the EQ (tolerance 2 dB for FIR truncation/windowing).
    let sr = 48_000.0;
    let mut eq = LinearPhaseEq::new(sr);
    let latency = eq.latency();
    let n = latency + 8192;
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    let freq = 1000.0;
    let omega = std::f32::consts::TAU * freq / sr;
    for i in 0..n {
        l[i] = (i as f32 * omega).sin() * 0.5;
        r[i] = l[i];
    }
    let mut bands = [BandConfig::off(); NUM_BANDS];
    bands[0] = BandConfig {
        enabled: true,
        band_type: BandType::Bell,
        freq_hz: 1000.0,
        q: 1.0,
        gain_db: -12.0,
    };
    eq.process_stereo(&mut l, &mut r, &bands);
    // Measure RMS of the settled tail.
    let tail_start = latency + 2048;
    let tail_len = n - tail_start;
    let mut sum_sq = 0.0_f64;
    for &s in &l[tail_start..n] {
        sum_sq += (s as f64) * (s as f64);
    }
    let rms = (sum_sq / tail_len as f64).sqrt() as f32;
    let input_rms = 0.5 / 2.0_f32.sqrt();
    let gain_db = 20.0 * (rms / input_rms).log10();
    assert!(
        (gain_db - -12.0).abs() < 2.0,
        "gain at 1 kHz = {gain_db} dB (expected -12 ± 2)"
    );
}

/// |H(f)| in dB of the EQ at `sr` for `bands`, measured from its
/// impulse response (the output of a unit impulse, DTFT'd directly).
fn eq_response_db(sr: f32, bands: &[BandConfig; NUM_BANDS], freqs: &[f32]) -> Vec<f64> {
    let mut eq = LinearPhaseEq::new(sr);
    let n = 2 * eq.latency() + 64;
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    l[0] = 1.0;
    r[0] = 1.0;
    eq.process_stereo(&mut l, &mut r, bands);
    assert!(l.iter().any(|v| v.abs() > 1e-3), "EQ impulse response is silent at {sr} Hz");
    freqs
        .iter()
        .map(|&f| {
            let (mut re, mut im) = (0.0_f64, 0.0_f64);
            for (i, &v) in l.iter().enumerate() {
                let ph = std::f64::consts::TAU * f as f64 * i as f64 / sr as f64;
                re += v as f64 * ph.cos();
                im -= v as f64 * ph.sin();
            }
            10.0 * (re * re + im * im).log10()
        })
        .collect()
}

/// DSP-06: the FIR was 4097 taps at every rate, so its time span — and
/// with it the low-frequency resolution — shrank with the sample rate: a
/// 30 Hz rumble high-pass passed rumble and cut 60 Hz by 2.5 dB at
/// 192 kHz. The response must match the 48 kHz design at every rate,
/// the passband must hold, and the latency must stay constant in ms.
#[test]
fn low_band_response_is_independent_of_sample_rate() {
    let mut bands = [BandConfig::off(); NUM_BANDS];
    bands[0] = BandConfig {
        enabled: true,
        band_type: BandType::HighPass,
        freq_hz: 30.0,
        q: 0.707,
        gain_db: 0.0,
    };
    let freqs = [10.0_f32, 20.0, 60.0, 120.0];
    let reference = eq_response_db(48_000.0, &bands, &freqs);
    let latency_ms_48 = LinearPhaseEq::new(48_000.0).latency() as f32 / 48.0;
    for sr in [96_000.0_f32, 192_000.0] {
        let got = eq_response_db(sr, &bands, &freqs);
        for ((f, g), want) in freqs.iter().zip(&got).zip(&reference) {
            assert!(
                (g - want).abs() < 1.0,
                "{sr} Hz: HP30 response at {f} Hz is {g:.2} dB, 48 kHz design gives {want:.2} dB"
            );
        }
        let target_60 = 20.0
            * (bands[0].to_biquad(sr).magnitude(60.0, sr) as f64).log10();
        assert!(
            (got[2] - target_60).abs() < 0.5,
            "{sr} Hz: 60 Hz passband {:.2} dB vs biquad target {target_60:.2} dB",
            got[2]
        );
        let latency_ms = LinearPhaseEq::new(sr).latency() as f32 / (sr / 1000.0);
        assert!(
            (latency_ms - latency_ms_48).abs() < 0.5,
            "{sr} Hz: latency {latency_ms:.1} ms vs {latency_ms_48:.1} ms at 48 kHz"
        );
    }
}
